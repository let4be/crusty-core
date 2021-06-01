use std::io::Read;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use flate2::read::GzDecoder;
use hyper::body::HttpBody;
use rand::{thread_rng, Rng};

#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{
	hyper_utils,
	resolver::{Adaptor as ResolverAdaptor, Resolver},
	types::*,
};

pub(crate) type HttpClient<R> = hyper::Client<HttpConnector<R>>;
pub(crate) type HttpConnector<R> =
	hyper_tls::HttpsConnector<hyper_utils::CountingConnector<hyper::client::HttpConnector<ResolverAdaptor<R>>>>;

pub(crate) trait ClientFactory<R: Resolver> {
	fn make(&self) -> (HttpClient<R>, hyper_utils::Stats);
}

pub(crate) struct TaskProcessor<JS: JobStateValues, TS: TaskStateValues, R: Resolver> {
	job:            ResolvedJob<JS, TS>,
	status_filters: StatusFilters<JS, TS>,
	load_filters:   LoadFilters<JS, TS>,
	task_expanders: Arc<TaskExpanders<JS, TS>>,
	term_on_error:  bool,

	tx:             Sender<JobUpdate<JS, TS>>,
	tasks_rx:       Receiver<Arc<Task>>,
	parse_tx:       Sender<ParserTask>,
	client_factory: Box<dyn ClientFactory<R> + Send + Sync + 'static>,
	resolver:       Arc<R>,
}

impl<JS: JobStateValues, TS: TaskStateValues, R: Resolver> TaskProcessor<JS, TS, R> {
	pub(crate) fn new(
		job: ResolvedJob<JS, TS>,
		tx: Sender<JobUpdate<JS, TS>>,
		tasks_rx: Receiver<Arc<Task>>,
		parse_tx: Sender<ParserTask>,
		client_factory: Box<dyn ClientFactory<R> + Send + Sync + 'static>,
		resolver: Arc<R>,
		term_on_error: bool,
	) -> TaskProcessor<JS, TS, R> {
		TaskProcessor {
			status_filters: job.rules.status_filters(),
			load_filters: job.rules.load_filters(),
			task_expanders: Arc::new(job.rules.task_expanders()),
			term_on_error,
			job,
			tx,
			tasks_rx,
			parse_tx,
			client_factory,
			resolver,
		}
	}

	async fn read(&self, body: &mut hyper::Response<hyper::Body>, encoding: String) -> anyhow::Result<Bytes> {
		let mut bytes = BytesMut::with_capacity(*self.job.settings.internal_read_buffer_size);

		while let Some(buf) = body.data().await {
			let buf = buf.context("error during reading")?;
			if buf.has_remaining() {
				if bytes.len() + buf.len() > *self.job.settings.max_response_size as usize {
					return Err(anyhow!("max response size reached: {}", *self.job.settings.max_response_size))
				}
				bytes.put(buf);
			}
		}

		if encoding.contains("gzip") || encoding.contains("deflate") {
			let mut buf: Vec<u8> = vec![];
			let _ = GzDecoder::new(bytes.reader()).read_to_end(&mut buf)?;
			return Ok(Bytes::from(buf))
		}

		Ok(bytes.freeze())
	}

	async fn resolve(&mut self, task: Arc<Task>) -> Result<ResolveData> {
		let t = Instant::now();

		let host =
			task.link.url.host_str().with_context(|| format!("cannot get host from {:?}", task.link.url.as_str()))?;

		let res = self.resolver.resolve(host).await.with_context(|| format!("cannot resolve host {} -> ip", host))?;

		Ok(ResolveData { metrics: ResolveMetrics { duration: t.elapsed() }, addrs: res.collect() })
	}

	async fn status(
		&mut self,
		task: Arc<Task>,
		client: &HttpClient<R>,
		is_head: bool,
	) -> Result<(HttpStatus, hyper::Response<hyper::Body>)> {
		let mut status_metrics = StatusMetrics { wait_duration: task.queued_at.elapsed(), ..Default::default() };

		let uri = hyper::Uri::from_str(task.link.url.as_str())
			.with_context(|| format!("cannot create http uri {}", &task.link.url))?;

		let t = Instant::now();

		let mut req = hyper::Request::builder();
		req = req.uri(uri).method(if is_head { hyper::Method::HEAD } else { hyper::Method::GET });

		for (n, vs) in &self.job.settings.custom_headers {
			for v in vs {
				req = req.header(n, v);
			}
		}

		let req_fut = client.request(req.body(hyper::Body::default()).context("could not construct http request")?);
		let resp = timeout(*self.job.settings.load_timeout, req_fut).await.map_err(|_| Error::LoadTimeout)?;

		let resp = resp.with_context(|| "cannot make http get")?;
		status_metrics.duration = t.elapsed();
		let rs = resp.status();

		let status = HttpStatus {
			started_at: t,
			code:       rs.as_u16() as i32,
			headers:    resp.headers().clone(),
			metrics:    status_metrics,
		};

		for filter in self.status_filters.iter() {
			let r = filter.accept(&mut self.job.ctx, &task, &status);
			match r {
				Err(ExtError::Term) => return Err(Error::StatusFilterTerm { name: filter.name() }),
				Err(ExtError::Other(err)) => {
					trace!("error in status filter {}: {:#}", filter.name(), &err);
					if self.term_on_error {
						return Err(Error::StatusFilterTermByError { name: filter.name(), source: err })
					}
				}
				Ok(_) => {}
			}
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
		let mut load_metrics = LoadMetrics { wait_duration: task.queued_at.elapsed(), ..Default::default() };

		let enc =
			String::from(resp.headers().get(http::header::CONTENT_ENCODING).map_or("", |h| h.to_str().unwrap_or("")));

		let body = self.read(&mut resp, enc).await.with_context(|| "cannot read http get response")?;

		load_metrics.read_size = stats.read();
		load_metrics.write_size = stats.write();

		load_metrics.duration = status.started_at.elapsed();

		for filter in self.load_filters.iter() {
			let r = filter.accept(&self.job.ctx, &task, &status, Box::new(body.clone().reader()));
			match r {
				Err(ExtError::Term) => return Err(Error::LoadFilterTerm { name: filter.name() }),
				Err(ExtError::Other(err)) => {
					trace!("error in status filter {}: {:#}", filter.name(), &err);
					if self.term_on_error {
						return Err(Error::LoadFilterTermByError { name: filter.name(), source: err })
					}
				}
				Ok(_) => {}
			}
		}

		let load_data = LoadData { metrics: load_metrics };
		Ok((load_data, Box::new(body.reader())))
	}

	async fn follow(
		&mut self,
		task: Arc<Task>,
		status: HttpStatus,
		reader: Box<dyn io::Read + Sync + Send>,
	) -> Result<FollowData> {
		let (parse_res_tx, parse_res_rx) = bounded_ch::<ParserResponse>(1);

		let task_state = Arc::new(Mutex::new(Some(TS::default())));
		let links = Arc::new(Mutex::new(Some(Vec::<Link>::new())));

		let payload = {
			let task = Arc::clone(&task);
			let task_expanders = Arc::clone(&self.task_expanders);
			let term_on_error = self.term_on_error;
			let mut job_ctx = self.job.ctx.clone();
			let task_state = Arc::clone(&task_state);
			let links = Arc::clone(&links);

			move || -> Result<FollowData> {
				let t = Instant::now();

				let document = select::document::Document::from_read(reader).context("cannot read html document")?;

				for expander in task_expanders.iter() {
					match expander.expand(&mut job_ctx, &task, &status, &document) {
						Ok(_) => {}
						Err(ExtError::Term) => return Err(Error::TaskExpanderTerm { name: expander.name() }),
						Err(ExtError::Other(err)) => {
							if term_on_error {
								return Err(Error::TaskExpanderTermByError { name: expander.name(), source: err })
							}
							trace!("error in task expander {}: {:#}", expander.name(), &err);
						}
					}
				}

				{
					let mut links = links.lock().unwrap();
					*links = Some(job_ctx._consume_links());
				}

				{
					let mut task_state = task_state.lock().unwrap();
					*task_state = Some(job_ctx.task_state);
				}

				Ok(FollowData { metrics: FollowMetrics { duration: t.elapsed() } })
			}
		};

		let _ = self
			.parse_tx
			.send_async(ParserTask { payload: Box::new(payload), time: Instant::now(), res_tx: parse_res_tx })
			.await;

		let parser_response = parse_res_rx.recv_async().await.context("cannot follow html document")?;
		let follow_data = parser_response.payload?;
		{
			let mut task_state = task_state.lock().unwrap();
			self.job.ctx.task_state = task_state.take().unwrap();
		}
		{
			let mut links = links.lock().unwrap();
			self.job.ctx.push_links(links.take().unwrap());
		}
		Ok(follow_data)
	}

	async fn process_task(
		&mut self,
		task: Arc<Task>,
		client: &HttpClient<R>,
		stats: &hyper_utils::Stats,
	) -> Result<JobProcessing> {
		let resolve_data = self.resolve(Arc::clone(&task)).await?;

		let mut status = JobProcessing {
			resolve_data,
			head_status: StatusResult::None,
			status: StatusResult::None,
			load: LoadResult::None,
			follow: FollowResult::None,
			links: vec![],
		};

		if task.link.target == LinkTarget::JustResolveDNS {
			return Ok(status)
		}

		if task.link.target == LinkTarget::Head
			|| task.link.target == LinkTarget::HeadLoad
			|| task.link.target == LinkTarget::HeadFollow
		{
			match self.status(Arc::clone(&task), &client, true).await {
				Ok((head_status, _)) => {
					status.head_status = StatusResult::Ok(head_status);
				}
				Err(err) => {
					status.head_status = StatusResult::Err(err);
					return Ok(status)
				}
			}
		};
		if task.link.target == LinkTarget::Head {
			return Ok(status)
		}

		let (get_status, resp) = match self.status(Arc::clone(&task), &client, false).await {
			Err(err) => {
				status.status = StatusResult::Err(err);
				return Ok(status)
			}
			Ok((get_status, resp)) => (get_status, resp),
		};

		let load_r = self.load(Arc::clone(&task), &get_status, stats, resp).await;
		status.status = StatusResult::Ok(get_status.clone());

		let reader = match load_r {
			Err(err) => {
				status.load = LoadResult::Err(err);
				return Ok(status)
			}
			Ok((load_data, reader)) => {
				status.load = LoadResult::Ok(load_data);
				reader
			}
		};

		if task.link.target == LinkTarget::Load || task.link.target == LinkTarget::HeadLoad {
			return Ok(status)
		}

		status.follow = self
			.follow(Arc::clone(&task), get_status, reader)
			.await
			.map(FollowResult::Ok)
			.unwrap_or_else(FollowResult::Err);

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
							ctx,
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
		})
		.instrument()
	}
}
