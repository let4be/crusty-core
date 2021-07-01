use crate::{
	_prelude::*,
	hyper_utils::Stats,
	task_filters,
	task_processor::{ClientFactory, HttpClient, TaskProcessor},
	types::*,
};

pub(crate) struct TaskScheduler<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> {
	job:          ResolvedJob<JS, TS, P>,
	task_filters: TaskFilters<JS, TS>,

	task_seq_num:  usize,
	pages_pending: usize,

	backend:   Box<dyn Backend<JS, TS> + Send + Sync + 'static>,
	update_tx: Sender<JobUpdate<JS, TS>>,
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> TaskScheduler<JS, TS, P> {
	pub(crate) fn new(
		job: ResolvedJob<JS, TS, P>,
		update_tx: Sender<JobUpdate<JS, TS>>,
		backend: Box<dyn Backend<JS, TS> + Send + Sync + 'static>,
	) -> TaskScheduler<JS, TS, P> {
		TaskScheduler {
			task_filters: job.rules.task_filters(),
			job,
			task_seq_num: 0,
			pages_pending: 0,
			backend,
			update_tx,
		}
	}

	fn schedule(&mut self, task: Arc<Task>) {
		self.pages_pending += 1;

		self.backend.schedule(task);
	}

	fn schedule_filter(&mut self, task: &mut Task) -> task_filters::Result {
		let action = if task.link.redirect > 0 { "[scheduling redirect]" } else { "[scheduling]" };

		let _span = span!(task = %task).entered();
		for filter in &mut self.task_filters {
			match filter.accept(&mut self.job.ctx, self.task_seq_num, task) {
				Ok(task_filters::Action::Accept) => continue,
				Ok(task_filters::Action::Skip) => {
					if task.is_root() {
						info!(action = "skip", filter_name = %filter.name(), action);
					} else {
						debug!(action = "skip", filter_name = %filter.name(), action);
					}
					return Ok(task_filters::Action::Skip)
				}
				Err(ExtError::Term { reason }) => {
					if task.is_root() {
						info!(action = "term", filter_name = %filter.name(), reason = reason, action);
					} else {
						debug!(action = "term", filter_name = %filter.name(), reason = reason, action);
					}
					return Err(ExtError::Term { reason })
				}
				Err(ExtError::Other(err)) => {
					debug!(filter_name = %filter.name(), "error during task filtering: {:#}", err);
					continue
				}
			}
		}

		debug!(action = "scheduled", action);
		Ok(task_filters::Action::Accept)
	}

	async fn process_task_response(&mut self, task_response: JobUpdate<JS, TS>, ignore_links: bool) {
		self.pages_pending -= 1;
		self.task_seq_num += 1;

		let mut links = self.job.ctx.consume_links();

		if let (JobStatus::Processing(Ok(ref r)), false) = (&task_response.status, ignore_links) {
			links.extend(r.links.clone());
		}

		let tasks: Vec<_> = links
			.iter()
			.filter_map(|link| Task::new(Arc::clone(link), &task_response.task).ok())
			.map(|mut task| (self.schedule_filter(&mut task), task))
			.take_while(|(r, _)| {
				if let Err(ExtError::Term { reason: _ }) = r {
					return false
				}
				true
			})
			.filter_map(|(r, task)| {
				if let Ok(task_filters::Action::Skip) = r {
					return None
				}
				Some(task)
			})
			.collect();

		for task in tasks {
			self.schedule(Arc::new(task));
		}

		// TODO(#13): figure a nice way to propagate errors in root tasks to Job Finished status
		/*if self.pages_pending < 1 && task_response.task.is_root() {

		}*/

		let _ = self.update_tx.send_async(task_response).await;
	}

	pub(crate) fn go<'a>(mut self) -> Result<TaskFut<'a, JobUpdate<JS, TS>>> {
		let mut root_task = Task::new_root(&self.job.url, self.job.link_target)?;

		Ok(TracingTask::new(span!(url = %self.job.url), async move {
			trace!(
				soft_timeout_ms = self.job.settings.job_soft_timeout.as_millis() as u32,
				hard_timeout_ms = self.job.settings.job_hard_timeout.as_millis() as u32,
				"Starting..."
			);

			let r = self.schedule_filter(&mut root_task);
			let root_task = Arc::new(root_task);
			if let Ok(task_filters::Action::Accept) = r {
				self.schedule(Arc::clone(&root_task));
			}

			let mut is_soft_timeout = false;
			let mut is_hard_timeout = false;
			let mut timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_soft_timeout);

			while self.pages_pending > 0 {
				tokio::select! {
					res = self.backend.next_update() => {
						self.process_task_response(res, is_soft_timeout).await;
					}

					_ = &mut timeout => {
						if is_soft_timeout {
							is_hard_timeout = true;
							break
						}
						is_soft_timeout = true;
						let jitter_ms = {
							let mut rng = thread_rng();
							Duration::from_millis(rng.gen_range(0..self.job.settings.job_hard_timeout_jitter.as_millis()) as u64)
						};
						timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_hard_timeout + jitter_ms);
					}
				}
			}

			trace!("Finishing..., pages remaining = {}", self.pages_pending);
			let status = JobStatus::Finished(if is_hard_timeout {
				Err(JobError::JobFinishedByHardTimeout)
			} else if is_soft_timeout {
				Err(JobError::JobFinishedBySoftTimeout)
			} else {
				Ok(JobFinished {})
			});
			Ok(JobUpdate { task: root_task, status, ctx: self.job.ctx })
		})
		.instrument())
	}
}

pub trait Backend<JS: JobStateValues, TS: TaskStateValues> {
	fn schedule(&mut self, task: Arc<Task>);
	fn next_update(&mut self) -> PinnedFutLT<JobUpdate<JS, TS>>;
}

pub trait Binding<JS: JobStateValues, TS: TaskStateValues> {
	fn next_task(&mut self) -> PinnedFutLT<Option<Arc<Task>>>;
	fn update(&mut self, update: JobUpdate<JS, TS>);
}

pub(crate) struct ChBackend<JS: JobStateValues, TS: TaskStateValues> {
	tasks_tx:      Sender<Arc<Task>>,
	tasks_rx:      Receiver<Arc<Task>>,
	job_update_tx: Sender<JobUpdate<JS, TS>>,
	job_update_rx: Receiver<JobUpdate<JS, TS>>,
}

pub(crate) struct ChBinding<JS: JobStateValues, TS: TaskStateValues> {
	tasks_rx:      Receiver<Arc<Task>>,
	job_update_tx: Sender<JobUpdate<JS, TS>>,
}

impl<JS: JobStateValues, TS: TaskStateValues> ChBackend<JS, TS> {
	pub(crate) fn new() -> Self {
		let (job_update_tx, job_update_rx) = unbounded_ch::<JobUpdate<JS, TS>>();
		let (tasks_tx, tasks_rx) = unbounded_ch::<Arc<Task>>();

		Self { tasks_tx, tasks_rx, job_update_tx, job_update_rx }
	}

	pub(crate) fn bind(&self) -> ChBinding<JS, TS> {
		ChBinding { tasks_rx: self.tasks_rx.clone(), job_update_tx: self.job_update_tx.clone() }
	}
}

impl<JS: JobStateValues, TS: TaskStateValues> Backend<JS, TS> for ChBackend<JS, TS> {
	fn schedule(&mut self, task: Arc<Task>) {
		let r = self.tasks_tx.send(task);
		if r.is_err() {
			panic!("cannot send task to tasks_tx! should never ever happen!")
		}
	}

	fn next_update(&mut self) -> PinnedFut<JobUpdate<JS, TS>> {
		let job_update_rx = self.job_update_rx.clone();
		Box::pin(async move { job_update_rx.recv_async().await.unwrap() })
	}
}

impl<JS: JobStateValues, TS: TaskStateValues> Binding<JS, TS> for ChBinding<JS, TS> {
	fn next_task(&mut self) -> PinnedFutLT<Option<Arc<Task>>> {
		let tasks_rx = self.tasks_rx.clone();
		Box::pin(async move {
			if tasks_rx.is_disconnected() {
				return None
			}
			tasks_rx.recv_async().await.ok()
		})
	}

	fn update(&mut self, update: JobUpdate<JS, TS>) {
		let _ = self.job_update_tx.send(update);
	}
}

pub(crate) struct SyncBackend<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> {
	tasks:     Vec<Arc<Task>>,
	processor: TaskProcessor<JS, TS, P>,
	client:    HttpClient,
	stats:     Stats,
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> SyncBackend<JS, TS, P> {
	pub(crate) fn new(processor: TaskProcessor<JS, TS, P>, client_factory: Box<dyn ClientFactory>) -> Self {
		let (client, stats) = client_factory.make();

		Self { tasks: vec![], processor, client, stats }
	}
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> Backend<JS, TS> for SyncBackend<JS, TS, P> {
	fn schedule(&mut self, task: Arc<Task>) {
		self.tasks.push(task);
	}

	fn next_update(&mut self) -> PinnedFutLT<JobUpdate<JS, TS>> {
		let task = self.tasks.pop().unwrap();

		let processor = &mut self.processor;
		let client = &self.client;
		let mut stats = &mut self.stats;
		Box::pin(async move { processor.invoke_task(task, client, &mut stats).await })
	}
}
