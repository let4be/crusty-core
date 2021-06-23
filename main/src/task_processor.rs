use std::panic;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use flate2::read::GzDecoder;
use hyper::body::HttpBody;
use io::Read;

#[allow(unused_imports)]
use crate::_prelude::*;
use crate::{
	hyper_utils,
	resolver::{Adaptor as ResolverAdaptor, Resolver},
	types::*,
};

pub(crate) type HttpClient = hyper::Client<HttpConnector>;
pub(crate) type HttpConnector =
	hyper_tls::HttpsConnector<hyper_utils::CountingConnector<hyper::client::HttpConnector<ResolverAdaptor>>>;

pub(crate) trait ClientFactory {
	fn make(&self) -> (HttpClient, hyper_utils::Stats);
}

pub(crate) struct TaskProcessor<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> {
	job:             ResolvedJob<JS, TS, P>,
	status_filters:  Arc<StatusFilters<JS, TS>>,
	load_filters:    Arc<LoadFilters<JS, TS>>,
	task_expanders:  Arc<TaskExpanders<JS, TS, P>>,
	document_parser: Arc<DocumentParser<P>>,

	tx:             Sender<JobUpdate<JS, TS>>,
	tasks_rx:       Receiver<Arc<Task>>,
	parse_tx:       Sender<ParserTask>,
	client_factory: Box<dyn ClientFactory + Send + Sync + 'static>,
	resolver:       Arc<Box<dyn Resolver>>,
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> TaskProcessor<JS, TS, P> {
	pub(crate) fn new(
		job: ResolvedJob<JS, TS, P>,
		tx: Sender<JobUpdate<JS, TS>>,
		tasks_rx: Receiver<Arc<Task>>,
		parse_tx: Sender<ParserTask>,
		client_factory: Box<dyn ClientFactory + Send + Sync + 'static>,
		resolver: Arc<Box<dyn Resolver>>,
	) -> TaskProcessor<JS, TS, P> {
		TaskProcessor {
			status_filters: Arc::new(job.rules.status_filters()),
			load_filters: Arc::new(job.rules.load_filters()),
			task_expanders: Arc::new(job.rules.task_expanders()),
			document_parser: job.rules.document_parser(),
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

	fn filter<T, Name: Fn(&T) -> &'static str, Filter: Fn(&mut JobCtx<JS, TS>, &T) -> ExtResult<()>>(
		ctx: &mut JobCtx<JS, TS>,
		kind: FilterKind,
		filters: &Arc<Vec<T>>,
		name_fn: Name,
		filter_fn: Filter,
	) -> Option<ExtStatusError> {
		for filter in filters.iter() {
			let r = panic::catch_unwind(panic::AssertUnwindSafe(|| filter_fn(ctx, filter)));

			if let Err(err) = r {
				return Some(ExtStatusError::Panic {
					kind,
					name: name_fn(filter),
					source: Arc::new(anyhow!("{:?}", err)),
				})
			}

			match r.unwrap() {
				Err(ExtError::Term { reason }) => {
					return Some(ExtStatusError::Term { kind, reason, name: name_fn(filter) })
				}

				Err(ExtError::Other(err)) => {
					trace!("error in {:?} {}: {:#}", kind, name_fn(filter), &err);
					return Some(ExtStatusError::Err { kind, name: name_fn(filter), source: Arc::new(err) })
				}

				Ok(_) => {}
			}
		}

		None
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
		client: &HttpClient,
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
		let resp = timeout(*self.job.settings.status_timeout, req_fut).await.map_err(|_| Error::StatusTimeout)?;

		let resp = resp.context("cannot make http get")?;
		status_metrics.duration = t.elapsed();
		let rs = resp.status();

		let mut status = HttpStatus {
			started_at: t,
			code:       rs.as_u16(),
			headers:    HeaderMap(resp.headers().clone()),
			metrics:    status_metrics,
			filter_err: None,
		};

		let status_filters = Arc::clone(&self.status_filters);
		status.filter_err = Self::filter(
			&mut self.job.ctx,
			if is_head { FilterKind::HeadStatusFilter } else { FilterKind::GetStatusFilter },
			&status_filters,
			|filter| filter.name(),
			|ctx, filter| filter.accept(ctx, &task, &status),
		);

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

		let read_fut = self.read(&mut resp, enc);
		let res = timeout(*self.job.settings.load_timeout, read_fut).await.map_err(|_| Error::LoadTimeout)?;
		let body = res.with_context(|| "cannot read http get response")?;

		load_metrics.read_size = stats.read();
		load_metrics.write_size = stats.write();
		load_metrics.duration = status.started_at.elapsed();

		let load_filters = Arc::clone(&self.load_filters);
		let filter_err = Self::filter(
			&mut self.job.ctx,
			FilterKind::LoadFilter,
			&load_filters,
			|filter| filter.name(),
			|ctx, filter| filter.accept(ctx, &task, &status, Box::new(body.clone().reader())),
		);

		let load_data = LoadData { metrics: load_metrics, filter_err };
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
		let links = Arc::new(Mutex::new(Some(Vec::<Arc<Link>>::new())));

		let payload = {
			let task = Arc::clone(&task);
			let task_expanders = Arc::clone(&self.task_expanders);
			let mut job_ctx = self.job.ctx.clone();
			let task_state = Arc::clone(&task_state);
			let links = Arc::clone(&links);
			let document_parser = Arc::clone(&self.document_parser);

			move || -> Result<FollowData> {
				let t = Instant::now();

				let document = document_parser(reader)?;

				let filter_err = Self::filter(
					&mut job_ctx,
					FilterKind::FollowFilter,
					&task_expanders,
					|filter| filter.name(),
					|ctx, filter| filter.expand(ctx, &task, &status, &document),
				);

				{
					let mut links = links.lock().unwrap();
					*links = Some(job_ctx.consume_links());
				}

				{
					let mut task_state = task_state.lock().unwrap();
					*task_state = Some(job_ctx.task_state);
				}

				let follow_data = FollowData { metrics: FollowMetrics { duration: t.elapsed() }, filter_err };
				Ok(follow_data)
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
			self.job.ctx.push_shared_links(links.take().unwrap().into_iter());
		}
		Ok(follow_data)
	}

	async fn process_task(
		&mut self,
		task: Arc<Task>,
		client: &HttpClient,
		stats: &hyper_utils::Stats,
	) -> Result<JobProcessing> {
		let resolve_data = self.resolve(Arc::clone(&task)).await?;

		let mut status = JobProcessing::new(resolve_data);

		if task.link.target == LinkTarget::JustResolveDNS {
			return Ok(status)
		}

		if task.link.target == LinkTarget::Head
			|| task.link.target == LinkTarget::HeadLoad
			|| task.link.target == LinkTarget::HeadFollow
		{
			match self.status(Arc::clone(&task), &client, true).await {
				Ok((head_status, _)) => {
					let done = head_status.filter_err.is_some();
					status.head_status = StatusResult(Some(Ok(head_status)));
					if done {
						return Ok(status)
					}
				}
				Err(err) => {
					status.head_status = StatusResult(Some(Err(err)));
					return Ok(status)
				}
			}
		};
		if task.link.target == LinkTarget::Head {
			return Ok(status)
		}

		let (get_status, resp) = match self.status(Arc::clone(&task), &client, false).await {
			Err(err) => {
				status.status = StatusResult(Some(Err(err)));
				return Ok(status)
			}
			Ok((get_status, resp)) => (get_status, resp),
		};

		let load_r = self.load(Arc::clone(&task), &get_status, stats, resp).await;
		{
			let done = get_status.filter_err.is_some();
			status.status = StatusResult(Some(Ok(get_status.clone())));
			if done {
				return Ok(status)
			}
		}

		let reader = match load_r {
			Err(err) => {
				status.load = LoadResult(Some(Err(err)));
				return Ok(status)
			}
			Ok((load_data, reader)) => {
				let done = load_data.filter_err.is_some();
				status.load = LoadResult(Some(Ok(load_data)));
				if done {
					return Ok(status)
				}

				reader
			}
		};

		if task.link.target == LinkTarget::Load || task.link.target == LinkTarget::HeadLoad {
			return Ok(status)
		}

		status.follow = self
			.follow(Arc::clone(&task), get_status, reader)
			.await
			.map(|follow_data| FollowResult(Some(Ok(follow_data))))
			.unwrap_or_else(|err| FollowResult(Some(Err(err))));

		Ok(status)
	}

	pub(crate) fn go<'a>(mut self, n: usize) -> TaskFut<'a> {
		TracingTask::new(span!(n=n, url=%self.job.url), async move {
			let (client, mut stats) = self.client_factory.make();

			let mut timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_hard_timeout);

			while let Ok(task) = self.tasks_rx.recv_async().await {
				if self.tasks_rx.is_disconnected() {
					break
				}

				stats.reset();

				tokio::select! {
					mut status_r = self.process_task(Arc::clone(&task), &client, &stats) => {
						if let Ok(ref mut status_data) = status_r {
							status_data.links.extend(self.job.ctx.consume_links());
						}

						let _ = self.tx.send_async(JobUpdate {
							task,
							status: JobStatus::Processing(status_r),
							ctx: self.job.ctx.clone(),
						}).await;
					}
					_ = &mut timeout => break
				}

				if self.job.settings.delay.as_millis() > 0 || self.job.settings.delay_jitter.as_millis() > 0 {
					let jitter_ms = {
						let mut rng = thread_rng();
						Duration::from_millis(rng.gen_range(0..self.job.settings.delay_jitter.as_millis()) as u64)
					};

					tokio::select! {
						_ = time::sleep(*self.job.settings.delay + jitter_ms) => {}
						_ = &mut timeout => break
					}
				}
			}

			Ok(())
		})
		.instrument()
	}
}
