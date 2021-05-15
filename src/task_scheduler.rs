#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    types::*,
    config,
    task_filters,
};

use std::sync::{Arc};

use url::Url;

pub(crate) struct TaskScheduler<T: JobContextValues> {
    settings: config::CrawlerSettings,
    task_filters: Vec<Box<dyn task_filters::TaskFilter<T> + Send + Sync>>,

    job_ctx: StdJobContext<T>,
    root_task: Task,
    task_seq_num: usize,
    pages_pending: usize,

    tasks_tx: Sender<Vec<Task>>,
    pub(crate) tasks_rx: Receiver<Vec<Task>>,
    pub(crate) job_update_tx: Sender<JobUpdate<T>>,
    job_update_rx: Receiver<JobUpdate<T>>,
    sub_tx: Sender<JobUpdate<T>>,
}

impl<T: JobContextValues> TaskScheduler<T> {
    pub(crate) fn new(
        url: &Url,
        settings: &config::CrawlerSettings,
        task_filters: Vec<Box<dyn task_filters::TaskFilter<T> + Send + Sync>>,
        job_context: StdJobContext<T>,
        sub_tx: Sender<JobUpdate<T>>
    ) -> Result<TaskScheduler<T>> {
        let root_task = Task::new_root(&url)?;

        let (job_update_tx, job_update_rx) = unbounded_ch::<JobUpdate<T>>();
        let (tasks_tx, tasks_rx) = unbounded_ch::<Vec<Task>>();

        Ok(TaskScheduler {
            settings: settings.clone(),
            task_filters,
            job_ctx: job_context,

            root_task,
            task_seq_num: 0,
            pages_pending: 0,

            tasks_tx,
            tasks_rx,
            job_update_tx,
            job_update_rx,
            sub_tx,
        })
    }

    async fn schedule(&mut self, tasks: Vec<Task>) {
        self.pages_pending += tasks.len();
        let _ = self.tasks_tx.send(tasks).await.unwrap();
    }

    fn schedule_filter(&mut self, task: &Task) -> task_filters::TaskFilterResult  {
        let action = if task.link.redirect > 0 { "[scheduling redirect]" } else { "[scheduling]" };

        for filter in &mut self.task_filters {
            match filter.accept(&mut self.job_ctx, self.task_seq_num, task) {
                task_filters::TaskFilterResult::Accept => continue,
                task_filters::TaskFilterResult::Skip => {
                    trace!(action = "skip", filter_name = filter.name(), action);
                    return task_filters::TaskFilterResult::Skip
                },
                task_filters::TaskFilterResult::Term => {
                    trace!(action = "term", filter_name = filter.name(), action);
                    return task_filters::TaskFilterResult::Term
                },
            }
        }

        trace!(action = "scheduled", action);
        task_filters::TaskFilterResult::Accept
    }

    async fn process_task_response(&mut self, task_response: JobUpdate<T>, ignore_links: bool) {
        self.pages_pending -= 1;
        self.task_seq_num += 1;

        if !ignore_links {
            match task_response.status {
                JobStatus::Processing(ref r) => {
                    if r.is_ok() {
                        let r = r.as_ref().unwrap();
                        let max_redirect = self.settings.max_redirect;

                        let tasks = r.collect_links()
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
                            })
                            .collect();

                        self.schedule(tasks).await;
                    }
                }
                _ => {}
            }
        }

        let _ = self.sub_tx.send(task_response).await;
    }

    pub(crate) fn go(&mut self) -> PinnedFut<JobUpdate<T>> {
        TracingTask::new(span!(Level::INFO), async move {
            trace!(
                soft_timeout_ms = self.settings.job_soft_timeout.as_millis() as u32,
                hard_timeout_ms = self.settings.job_hard_timeout.as_millis() as u32,
                "Starting..."
            );

            self.schedule(vec![self.root_task.clone()]).await;

            let mut is_soft_timeout = false;
            let mut is_hard_timeout = false;
            let mut timeout = self.job_ctx.timeout_remaining(*self.settings.job_soft_timeout);

            while self.pages_pending > 0 {
                tokio::select! {
                    res = self.job_update_rx.recv() => {
                        self.process_task_response(res.unwrap(), false).await;
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
            Ok(JobUpdate {
                task: Arc::new(self.root_task.clone()),
                status: JobStatus::Finished(JobData { err }),
                context: self.job_ctx.clone(),
            })
        }).instrument()
    }
}