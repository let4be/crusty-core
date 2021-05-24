#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{types::*, task_filters};

pub(crate) struct TaskScheduler<JS: JobStateValues, TS: TaskStateValues> {
    job: ResolvedJob<JS, TS>,
    task_filters: TaskFilters<JS, TS>,

    task_seq_num: usize,
    pages_pending: usize,

    tasks_tx: Sender<Arc<Task>>,
    pub(crate) tasks_rx: Receiver<Arc<Task>>,
    pub(crate) job_update_tx: Sender<JobUpdate<JS, TS>>,
    job_update_rx: Receiver<JobUpdate<JS, TS>>,
    update_tx: Sender<JobUpdate<JS, TS>>,
}

impl<JS: JobStateValues, TS: TaskStateValues> TaskScheduler<JS, TS> {
    pub(crate) fn new(
        job: ResolvedJob<JS, TS>,
        update_tx: Sender<JobUpdate<JS, TS>>
    ) -> TaskScheduler<JS, TS> {
        let (job_update_tx, job_update_rx) = unbounded_ch::<JobUpdate<JS, TS>>();
        let (tasks_tx, tasks_rx) = unbounded_ch::<Arc<Task>>();

        TaskScheduler {
            task_filters: job.rules.task_filters(),
            job,

            task_seq_num: 0,
            pages_pending: 0,

            tasks_tx,
            tasks_rx,
            job_update_tx,
            job_update_rx,
            update_tx,
        }
    }

    fn schedule(&mut self, task: Arc<Task>) {
        self.pages_pending += 1;

        //we use try_send as it's faster and channel is unbounded
        let r = self.tasks_tx.try_send(task);
        if r.is_err() {
            panic!("cannot send task to tasks_tx! should never ever happen!")
        }
    }

    fn schedule_filter(&mut self, task: &mut Task) -> task_filters::Action {
        let action = if task.link.redirect > 0 { "[scheduling redirect]" } else { "[scheduling]" };

        for filter in &mut self.task_filters {
            match filter.accept(&mut self.job.ctx, self.task_seq_num, task) {
                task_filters::Action::Accept => continue,
                task_filters::Action::Skip => {
                    if task.is_root() {
                        warn!(action = "skip", filter_name = filter.name(), action);
                    } else {
                        trace!(action = "skip", filter_name = filter.name(), action);
                    }
                    return task_filters::Action::Skip
                },
                task_filters::Action::Term => {
                    if task.is_root() {
                        warn!(action = "term", filter_name = filter.name(), action);
                    } else {
                        trace!(action = "term", filter_name = filter.name(), action);
                    }
                    return task_filters::Action::Term
                },
            }
        }

        trace!(action = "scheduled", action);
        task_filters::Action::Accept
    }

    async fn process_task_response(&mut self, task_response: JobUpdate<JS, TS>, ignore_links: bool) {
        self.pages_pending -= 1;
        self.task_seq_num += 1;

        if let (JobStatus::Processing(ref r), false) = (&task_response.status, ignore_links) {
            let tasks :Vec<_> = r.links.iter()
                .filter_map(|link| {
                    Task::new(Arc::clone(link), &task_response.task).ok()
                })
                .map(|mut task| {
                    (self.schedule_filter(&mut task), task)
                })
                .take_while(|(r, _)|{
                    *r != task_filters::Action::Term
                })
                .filter_map(|(r, task)| {
                    if r == task_filters::Action::Skip {
                        return None
                    }
                    Some(task)
                })
                .collect();

            for task in tasks {
                self.schedule(Arc::new(task));
            }
        }

        let _ = self.update_tx.send(task_response).await;
    }

    pub(crate) fn go<'a>(mut self) -> Result<PinnedTask<'a, JobUpdate<JS, TS>>> {
        let mut root_task = Task::new_root(&self.job.url)?;

        Ok(TracingTask::new(span!(Level::INFO), async move {
            trace!(
                soft_timeout_ms = self.job.settings.job_soft_timeout.as_millis() as u32,
                hard_timeout_ms = self.job.settings.job_hard_timeout.as_millis() as u32,
                "Starting..."
            );

            let is_accepted = self.schedule_filter(&mut root_task) == task_filters::Action::Accept;
            let root_task = Arc::new(root_task);
            if is_accepted {
                self.schedule(Arc::clone(&root_task));
            }

            let mut is_soft_timeout = false;
            let mut is_hard_timeout = false;
            let mut timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_soft_timeout);

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
                        timeout = self.job.ctx.timeout_remaining(*self.job.settings.job_hard_timeout);
                    }
                }
            }

            trace!("Finishing..., pages remaining = {}", self.pages_pending);
            let res = if is_hard_timeout {
                Err(JobError::JobFinishedByHardTimeout)
            } else if is_soft_timeout {
                Err(JobError::JobFinishedBySoftTimeout)
            } else {
                Ok(JobData{})
            };
            Ok(JobUpdate {
                task: root_task,
                status: JobStatus::Finished(res),
                context: self.job.ctx,
            })
        }).instrument())
    }
}