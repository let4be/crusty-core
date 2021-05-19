#[allow(unused_imports)]
use crate::prelude::*;
use crate::{types::*, config, task_filters, task_filters::TaskFilterResult};

pub(crate) struct TaskScheduler<JS: JobStateValues, TS: TaskStateValues> {
    task_filters: TaskFilters<JS, TS>,
    settings: config::CrawlerSettings,
    job_ctx: JobContext<JS, TS>,

    root_task: Task,
    task_seq_num: usize,
    pages_pending: usize,

    tasks_tx: Sender<Task>,
    pub(crate) tasks_rx: Receiver<Task>,
    pub(crate) job_update_tx: Sender<JobUpdate<JS, TS>>,
    job_update_rx: Receiver<JobUpdate<JS, TS>>,
    update_tx: Sender<JobUpdate<JS, TS>>,
}

impl<JS: JobStateValues, TS: TaskStateValues> TaskScheduler<JS, TS> {
    pub(crate) fn new(
        url: &Url,
        rules: Arc<BoxedJobRules<JS, TS>>,
        settings: &config::CrawlerSettings,
        job_context: JobContext<JS, TS>,
        update_tx: Sender<JobUpdate<JS, TS>>
    ) -> Result<TaskScheduler<JS, TS>> {
        let root_task = Task::new_root(&url)?;

        let (job_update_tx, job_update_rx) = unbounded_ch::<JobUpdate<JS, TS>>();
        let (tasks_tx, tasks_rx) = unbounded_ch::<Task>();

        Ok(TaskScheduler {
            settings: settings.clone(),
            task_filters: rules.task_filters(),
            job_ctx: job_context,

            root_task,
            task_seq_num: 0,
            pages_pending: 0,

            tasks_tx,
            tasks_rx,
            job_update_tx,
            job_update_rx,
            update_tx,
        })
    }

    fn schedule(&mut self, task: Task) {
        self.pages_pending += 1;

        //we use try_send as it's faster and channel is unbounded
        let r = self.tasks_tx.try_send(task);
        if r.is_err() {
            panic!("cannot send task to tasks_tx! should never ever happen!")
        }
    }

    fn schedule_filter(&mut self, task: &Task) -> task_filters::TaskFilterResult  {
        let action = if task.link.redirect > 0 { "[scheduling redirect]" } else { "[scheduling]" };

        for filter in &mut self.task_filters {
            match filter.accept(&mut self.job_ctx, self.task_seq_num, task) {
                task_filters::TaskFilterResult::Accept => continue,
                task_filters::TaskFilterResult::Skip => {
                    if task.is_root() {
                        warn!(action = "skip", filter_name = filter.name(), action);
                    } else {
                        trace!(action = "skip", filter_name = filter.name(), action);
                    }
                    return task_filters::TaskFilterResult::Skip
                },
                task_filters::TaskFilterResult::Term => {
                    if task.is_root() {
                        warn!(action = "term", filter_name = filter.name(), action);
                    } else {
                        trace!(action = "term", filter_name = filter.name(), action);
                    }
                    return task_filters::TaskFilterResult::Term
                },
            }
        }

        trace!(action = "scheduled", action);
        task_filters::TaskFilterResult::Accept
    }

    async fn process_task_response(&mut self, task_response: JobUpdate<JS, TS>, ignore_links: bool) {
        self.pages_pending -= 1;
        self.task_seq_num += 1;

        if !ignore_links {
            if let JobStatus::Processing(Ok(ref r)) = task_response.status {
                let max_redirect = self.settings.max_redirect;

                let tasks :Vec<Task> = r.collect_links()
                    .filter_map(|link| {
                        if link.redirect > max_redirect {
                            info!(url = link.url.as_str(), "[max redirect]");
                            return None
                        }
                        Task::new(link.clone(), &task_response.task).ok()
                    })
                    .map(|task| {
                        (self.schedule_filter(&task), task)
                    })
                    .take_while(|r|{
                        r.0 != task_filters::TaskFilterResult::Term
                    })
                    .filter_map(|v| {
                        let (r, task) = v;
                        if r == task_filters::TaskFilterResult::Skip {
                            return None
                        }
                        Some(task)
                    }).collect();

                for task in tasks {
                    self.schedule(task);
                }
            }
        }

        let _ = self.update_tx.send(task_response).await;
    }

    pub(crate) fn go(&mut self) -> PinnedTask<JobUpdate<JS, TS>> {
        TracingTask::new(span!(Level::INFO), async move {
            trace!(
                soft_timeout_ms = self.settings.job_soft_timeout.as_millis() as u32,
                hard_timeout_ms = self.settings.job_hard_timeout.as_millis() as u32,
                "Starting..."
            );

            let root_task= self.root_task.clone();
            if self.schedule_filter(&root_task) == TaskFilterResult::Accept {
                self.schedule(root_task);
            }

            let mut is_soft_timeout = false;
            let mut is_hard_timeout = false;
            let mut timeout = self.job_ctx.timeout_remaining(*self.settings.job_soft_timeout);

            while self.pages_pending > 0 {
                tokio::select! {
                    res = self.job_update_rx.recv() => {
                        self.process_task_response(res.unwrap(), is_soft_timeout).await;
                    }

                    _ = &mut timeout => {
                        if is_soft_timeout {
                            is_hard_timeout = true;
                            break
                        }
                        is_soft_timeout = true;
                        timeout = self.job_ctx.timeout_remaining(*self.settings.job_hard_timeout);
                    }
                }
            }

            trace!("Finishing..., pages remaining = {}", self.pages_pending);
            let err = if is_hard_timeout {
                Some(JobError::JobFinishedByHardTimeout)
            } else if is_soft_timeout {
                Some(JobError::JobFinishedBySoftTimeout)
            } else {
                None
            };
            let ctx = self.job_ctx.clone();
            {
                let mut job_state = ctx.job_state.lock().unwrap();
                job_state.finalize();
            }
            Ok(JobUpdate {
                task: Arc::new(self.root_task.clone()),
                status: JobStatus::Finished(JobData { err }),
                context: ctx,
            })
        }).instrument()
    }
}