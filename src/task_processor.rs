#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{types::*, status_filters, load_filters, hyper_utils, resolver::{Resolver, Adaptor as ResolverAdaptor}};

use bytes::{Buf, BufMut, BytesMut};
use flate2::read::GzDecoder;
use hyper::body::HttpBody;
use rand::{Rng, thread_rng};

pub(crate) type HttpClient<R> = hyper::Client<HttpConnector<R>>;
pub(crate) type HttpConnector<R> = hyper_tls::HttpsConnector<hyper_utils::CountingConnector<hyper::client::HttpConnector<ResolverAdaptor<R>>>>;

pub(crate) trait ClientFactory<R: Resolver> {
    fn make(&self) -> (HttpClient<R>, hyper_utils::Stats);
}

pub(crate) struct TaskProcessor<JS: JobStateValues, TS: TaskStateValues, R: Resolver> {
    job: ResolvedJob<JS, TS>,
    status_filters: StatusFilters<JS, TS>,
    load_filters: LoadFilters<JS, TS>,
    task_expanders: Arc<TaskExpanders<JS, TS>>,

    tx: Sender<JobUpdate<JS, TS>>,
    tasks_rx: Receiver<Arc<Task>>,
    parse_tx: Sender<ParserTask>,
    client_factory: Box<dyn ClientFactory<R> + Send + Sync + 'static>,
    resolver: Arc<R>
}

impl<JS: JobStateValues, TS: TaskStateValues, R: Resolver> TaskProcessor<JS, TS, R>
{
    pub(crate) fn new(
        job: ResolvedJob<JS, TS>,
        tx: Sender<JobUpdate<JS, TS>>,
        tasks_rx: Receiver<Arc<Task>>,
        parse_tx: Sender<ParserTask>,
        client_factory: Box<dyn ClientFactory<R> + Send + Sync + 'static>,
        resolver: Arc<R>
    ) -> TaskProcessor<JS, TS, R> {
        TaskProcessor {
            status_filters: job.rules.status_filters(),
            load_filters: job.rules.load_filters(),
            task_expanders: Arc::new(job.rules.task_expanders()),
            job,
            tx,
            tasks_rx,
            parse_tx,
            client_factory,
            resolver
        }
    }

    async fn read(&self, body: &mut hyper::Response<hyper::Body>) -> Result<BytesMut> {
        let mut bytes = BytesMut::with_capacity(*self.job.settings.internal_read_buffer_size);

        while let Some(buf) = body.data().await {
            let buf = buf.context("")?;
            if buf.has_remaining() {
                if bytes.len() + buf.len() > *self.job.settings.max_response_size as usize {
                    return Err(anyhow!("max response size reached: {}", *self.job.settings.max_response_size).into());
                }
                bytes.put(buf);
            }
        }

        Ok(bytes)
    }

    async fn resolve(
        &mut self,
        task: Arc<Task>,
    ) -> Result<ResolveData> {
        let t = Instant::now();

        let host = task.link.url.host_str()
            .with_context(|| format!("cannot get host from {:?}", task.link.url.as_str()))?;

        let res = self.resolver
            .resolve(host)
            .await
            .with_context(|| format!("cannot resolve host {} -> ip", host))?;

        Ok(ResolveData {
            metrics: ResolveMetrics{
                resolve_dur: t.elapsed(),
            },
            addrs: res.collect()
        })
    }

    async fn status(
        &mut self,
        task: Arc<Task>,
        client: &HttpClient<R>,
        is_head: bool,
    ) -> Result<(HttpStatus, hyper::Response<hyper::Body>)> {
        let mut status_metrics = StatusMetrics{ wait_dur: task.queued_at.elapsed(), ..Default::default()};

        let uri = hyper::Uri::from_str(task.link.url.as_str())
            .with_context(|| format!("cannot create http uri {}", &task.link.url))?;

        let t = Instant::now();

        let mut req = hyper::Request::builder();
        req = req.uri(uri).method(if is_head {hyper::Method::HEAD} else {hyper::Method::GET});

        for (n, v) in &self.job.settings.custom_headers {
            req = req.header(n, v[0].clone());
        }

        let timeout = time::sleep(*self.job.settings.load_timeout);

        let resp;
        tokio::select! {
            r = client.request(req.body(hyper::Body::default()).unwrap()) => {
                resp = r;
            },
            _ = timeout => {
                return Err(Error::LoadTimeout)
            }
        }

        let resp = resp.with_context(|| "cannot make http get")?;
        status_metrics.status_dur = t.elapsed();
        let rs = resp.status();

        let status = HttpStatus {
            started_processing_on: t,
            status_code: rs.as_u16() as i32,
            headers: resp.headers().clone(),
            status_metrics,
        };

        let mut term_by = None;
        for filter in self.status_filters.iter() {
            let r = filter.accept(&mut self.job.ctx, &task, &status);
            match r {
                status_filters::Action::Term => {
                    term_by = Some(filter.name());
                    break
                },
                status_filters::Action::Skip => {}
            }
        }

        if term_by.is_some() {
            return Err(Error::FilterTerm {name: term_by.unwrap().into()})
        }

        Ok((status, resp))
    }

    async fn load(
        &mut self,
        task: Arc<Task>,
        status: &HttpStatus,
        stats: &hyper_utils::Stats,
        mut resp: hyper::Response<hyper::Body>,
    ) -> Result<(LoadData, Box<dyn io::Read + Sync + Send>)> {
        let mut load_metrics = LoadMetrics {..Default::default()};

        let body = self.read(&mut resp).await.with_context(|| "cannot read http get response")?;

        load_metrics.read_size = stats.read();
        load_metrics.write_size = stats.write();

        let enc = resp.headers()
            .get(http::header::CONTENT_ENCODING)
            .map_or("",|h| h.to_str().unwrap_or(""));

        let reader : Box<dyn io::Read + Sync + Send> = if enc.contains("gzip") || enc.contains("deflate") {
            Box::new(GzDecoder::new(body.reader()))
        } else {
            Box::new(body.reader())
        };

        load_metrics.load_dur = status.started_processing_on.elapsed();

        let mut term_by = None;
        for filter in self.load_filters.iter() {
            let r = filter.accept(&self.job.ctx, &task, &status);
            match r {
                load_filters::Action::Term => {
                    term_by = Some(filter.name());
                    break
                }
                load_filters::Action::Skip => {}
            }
        }

        if term_by.is_some() {
            return Err(Error::FilterTerm {name: term_by.unwrap().into()})
        }

        let load_data = LoadData{
            metrics: load_metrics,
        };
        Ok((load_data, reader))
    }

    async fn follow(
        &mut self,
        task: Arc<Task>,
        status: HttpStatus,
        reader: Box<dyn io::Read + Sync + Send>,
    ) -> Result<FollowData> {
        let (parse_res_tx, parse_res_rx) = bounded_ch::<ParserResponse>(1);

        let payload = {
            let task = Arc::clone(&task);
            let task_expanders = Arc::clone(&self.task_expanders);
            let mut job_ctx = self.job.ctx.clone();

            move || -> Result<(FollowData, Vec<Arc<Link>>)> {
                let t = Instant::now();

                let document = select::document::Document::from_read(reader).context("cannot read html document")?;

                for h in task_expanders.iter() {
                    h.expand(&mut job_ctx, &task, &status, &document);
                }

                Ok((FollowData{
                    metrics: FollowMetrics{
                        parse_dur: t.elapsed(),
                    },
                }, job_ctx.consume_links()))
            }
        };

        let _ = self.parse_tx.send_async(ParserTask{
            payload: Box::new(payload),
            time: Instant::now(),
            res_tx: parse_res_tx,
        }).await;

        let parser_response = parse_res_rx.recv_async().await.context("cannot follow html document")?;
        let (follow_data, links) = parser_response.res?;
        self.job.ctx.push_arced_links(links);
        Ok(follow_data)
    }

    async fn process_task(&mut self, task: Arc<Task>, client: &HttpClient<R>, stats: &hyper_utils::Stats) -> Result<JobProcessing> {
        let resolve_data = self.resolve(Arc::clone(&task)).await?;

        let mut status = JobProcessing {
            resolve_data,
            head_status: StatusResult::None,
            status: StatusResult::None,
            load_data: LoadResult::None,
            follow_data: FollowResult::None,
            links: vec![],
        };

        if task.link.target == LinkTarget::JustResolveDNS {
            return Ok(status)
        }

        if task.link.target == LinkTarget::Head || task.link.target == LinkTarget::HeadLoad || task.link.target == LinkTarget::HeadFollow {
            let status_r = self.status(Arc::clone(&task), &client, true).await;
            if status_r.is_err() {
                status.head_status = StatusResult::Err(status_r.err().unwrap());
                return Ok(status);
            }
            let (head_status, _) = status_r.unwrap();
            status.head_status = StatusResult::Ok(head_status);
        };
        if task.link.target == LinkTarget::Head {
            return Ok(status);
        }

        let status_r = self.status(Arc::clone(&task), &client, false).await;
        if status_r.is_err() {
            status.status = StatusResult::Err(status_r.err().unwrap());
            return Ok(status);
        }

        let (get_status, resp) = status_r.unwrap();

        let load_r = self.load(Arc::clone(&task), &get_status, stats, resp).await;
        status.status = StatusResult::Ok(get_status.clone());

        if load_r.is_err() {
            status.load_data = LoadResult::Err(load_r.err().unwrap());
            return Ok(status);
        }

        let (load_data, reader) = load_r.unwrap();
        status.load_data = LoadResult::Ok(load_data);
        if task.link.target == LinkTarget::Load || task.link.target == LinkTarget::HeadLoad {
            return Ok(status);
        }

        let follow_r = self.follow(Arc::clone(&task), get_status, reader).await;
        if follow_r.is_err() {
            status.follow_data = FollowResult::Err(follow_r.err().unwrap());
            return Ok(status);
        }
        let follow_data = follow_r.unwrap();
        status.follow_data = FollowResult::Ok(follow_data);
        Ok(status)
    }

    pub(crate) fn go(&mut self, n: usize) -> PinnedTask {
        TracingTask::new(span!(n=n, url=%self.job.url), async move {
            let (client, mut stats) = self.client_factory.make();

            while let Ok(task) = self.tasks_rx.recv_async().await {
                if self.tasks_rx.is_disconnected() {
                    break
                }

                let timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_hard_timeout);

                stats.reset();

                tokio::select! {
                    mut status_r = self.process_task(Arc::clone(&task), &client, &stats) => {
                        let ctx = self.job.ctx.clone();
                        if let Ok(ref mut status_data) = status_r {
                            status_data.links.extend(self.job.ctx.consume_links());
                        }

                        let _ = self.tx.send_async(JobUpdate {
                            task,
                            status: JobStatus::Processing(status_r),
                            context: ctx,
                        }).await;
                    }
                    _ = timeout => break
                }

                if self.job.settings.delay.as_millis() > 0 || self.job.settings.delay_jitter.as_millis() > 0 {
                    let jitter_ms = {
                        let mut rng = thread_rng();
                        Duration::from_millis(rng.gen_range(0..self.job.settings.delay_jitter.as_millis()) as u64)
                    };

                    let timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_hard_timeout);
                    tokio::select! {
                        _ = time::sleep(*self.job.settings.delay+jitter_ms) => {}
                        _ = timeout => break
                    }
                }
            }

            Ok(())
        }).instrument()
    }
}