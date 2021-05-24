#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{types::*, status_filters, load_filters, hyper_utils};

use bytes::{Buf, BufMut, BytesMut};
use flate2::read::GzDecoder;
use hyper::body::HttpBody;
use rand::{Rng, thread_rng};

pub(crate) trait LikeHttpConnector: hyper::client::connect::Connect + Clone + Send + Sync + 'static {}
impl<T: hyper::client::connect::Connect + Clone + Send + Sync + 'static> LikeHttpConnector for T{}

pub(crate) type ClientFactory<C> = Box<dyn Fn() -> (hyper::Client<C>, hyper_utils::Stats) + Send + Sync + 'static>;

pub(crate) struct TaskProcessor<JS: JobStateValues, TS: TaskStateValues, C: LikeHttpConnector> {
    job: ResolvedJob<JS, TS>,
    status_filters: StatusFilters<JS, TS>,
    load_filters: LoadFilters<JS, TS>,
    task_expanders: Arc<TaskExpanders<JS, TS>>,

    tx: Sender<JobUpdate<JS, TS>>,
    tasks_rx: Receiver<Arc<Task>>,
    parse_tx: Sender<ParserTask>,
    client_factory: ClientFactory<C>,
}

impl<JS: JobStateValues, TS: TaskStateValues, C: LikeHttpConnector> TaskProcessor<JS, TS, C>
{
    pub(crate) fn new(
        job: ResolvedJob<JS, TS>,
        tx: Sender<JobUpdate<JS, TS>>,
        tasks_rx: Receiver<Arc<Task>>,
        parse_tx: Sender<ParserTask>,
        client_factory: ClientFactory<C>,
    ) -> TaskProcessor<JS, TS, C> {
        TaskProcessor {
            status_filters: job.rules.status_filters(),
            load_filters: job.rules.load_filters(),
            task_expanders: Arc::new(job.rules.task_expanders()),
            job,
            tx,
            tasks_rx,
            parse_tx,
            client_factory,
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

    async fn status(
        &mut self,
        task: Arc<Task>,
        client: &hyper::Client<C>,
        is_head: bool,
    ) -> Result<(HttpStatus, hyper::Response<hyper::Body>)> {
        let mut status_metrics = StatusMetrics{wait_time: task.queued_at.elapsed(), ..Default::default()};

        let uri = hyper::Uri::from_str(task.link.url.as_str())
            .with_context(|| format!("cannot create http uri {}", task.link.url.as_str()))?;

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
        status_metrics.status_time = t.elapsed();
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
    ) -> Result<(LoadData, Box<dyn std::io::Read + Sync + Send>)> {
        let mut load_metrics = LoadMetrics {..Default::default()};

        let body = self.read(&mut resp).await.with_context(|| "cannot read http get response")?;

        load_metrics.read_size = stats.read();
        load_metrics.write_size = stats.write();

        let mut enc = String::from("");
        let content_encoding_header = resp.headers().get(http::header::CONTENT_ENCODING);
        if content_encoding_header.is_some() {
            let s = content_encoding_header.unwrap().to_str();
            if s.is_ok() {
                enc = String::from(s.unwrap());
            }
        }

        let reader;
        if enc.contains("gzip") || enc.contains("deflate") {
            reader = Box::new(GzDecoder::new(body.reader())) as Box<dyn std::io::Read + Sync + Send>
        } else {
            reader = Box::new(body.reader()) as Box<dyn std::io::Read + Sync + Send>
        }

        load_metrics.load_time = status.started_processing_on.elapsed();

        let mut term_by = None;
        for filter in self.load_filters.iter() {
            let r = filter.accept(&self.job.ctx, &task, &status);
            match r {
                load_filters::LoadFilterAction::Term => {
                    term_by = Some(filter.name());
                    break
                }
                load_filters::LoadFilterAction::Skip => {}
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
        reader: Box<dyn std::io::Read + Sync + Send>,
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
                        parse_time: t.elapsed(),
                    },
                }, job_ctx.consume_links()))
            }
        };

        let _ = self.parse_tx.send(ParserTask{
            payload: Box::new(payload),
            time: Instant::now(),
            res_tx: parse_res_tx,
        }).await;

        let parser_response = parse_res_rx.recv().await.context("cannot follow html document")?;
        let (follow_data, links) = parser_response.res?;
        self.job.ctx.push_arced_links(links);
        Ok(follow_data)
    }


    async fn process_task(&mut self, task: Arc<Task>, client: &hyper::Client<C>, stats: &hyper_utils::Stats) -> StatusData{
        let mut status = StatusData{
            head_status: StatusResult::None,
            status: StatusResult::None,
            load_data: LoadResult::None,
            follow_data: FollowResult::None,
            links: vec![],
        };
        if task.link.target == LinkTarget::Head ||
            task.link.target == LinkTarget::HeadLoad ||
            task.link.target == LinkTarget::HeadFollow
        {
            let status_r = self.status(Arc::clone(&task), &client, true).await;
            if status_r.is_err() {
                status.head_status = StatusResult::Err(status_r.err().unwrap());
                return status;
            }
            let (head_status, _) = status_r.unwrap();
            status.head_status = StatusResult::Ok(head_status);
        };
        if task.link.target == LinkTarget::Head {
            return status;
        }

        let status_r = self.status(Arc::clone(&task), &client, false).await;
        if status_r.is_err() {
            status.status = StatusResult::Err(status_r.err().unwrap());
            return status;
        }

        let (get_status, resp) = status_r.unwrap();

        let load_r = self.load(Arc::clone(&task), &get_status, stats, resp).await;
        status.status = StatusResult::Ok(get_status.clone());

        if load_r.is_err() {
            status.load_data = LoadResult::Err(load_r.err().unwrap());
            return status;
        }

        let (load_data, reader) = load_r.unwrap();
        status.load_data = LoadResult::Ok(load_data);
        if task.link.target == LinkTarget::Load || task.link.target == LinkTarget::HeadLoad {
            return status;
        }

        let follow_r = self.follow(Arc::clone(&task), get_status, reader).await;
        if follow_r.is_err() {
            status.follow_data = FollowResult::Err(follow_r.err().unwrap());
            return status;
        }
        let follow_data = follow_r.unwrap();
        status.follow_data = FollowResult::Ok(follow_data);
        status
    }

    pub(crate) fn go(&mut self, n: usize) -> PinnedTask {
        TracingTask::new(span!(Level::INFO, n=n, url=self.job.url.as_str()), async move {
            let (client, mut stats) = (self.client_factory)();

            while let Ok(task) = self.tasks_rx.recv().await {
                if self.tasks_rx.is_closed() {
                    break
                }

                let timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_hard_timeout);

                stats.reset();

                tokio::select! {
                    mut status_data = self.process_task(Arc::clone(&task), &client, &stats) => {
                        let ctx = self.job.ctx.clone();
                        status_data.links.extend(self.job.ctx.consume_links());

                        let _ = self.tx.send(JobUpdate {
                            task,
                            status: JobStatus::Processing(status_data),
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