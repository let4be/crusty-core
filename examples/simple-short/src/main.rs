use crusty_core::prelude::*;

#[derive(Debug, Clone, Default)]
pub struct JobState {
    sum_title_len: usize
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String
}

pub struct DataExtractor {}
impl TaskExpander<JobState, TaskState> for DataExtractor {
    fn expand(&self, ctx: &mut JobCtx<JobState, TaskState>, _: &Task, _: &HttpStatus, doc: &Document) {
        let title = doc.find(Name("title")).next().map(|v|v.text());
        if let Some(title) = title {
            ctx.task_state.lock().unwrap().title = title.clone();
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let crawler = Crawler::new_default()?;

    let settings = config::CrawlingSettings::default();
    let rules = CrawlingRules::default().with_task_expander(|| DataExtractor{} );

    let job = Job::new("https://example.com", settings, rules, JobState::default())?;
    for r in crawler.iter(job) {
        println!("- {}, task state: {:?}", r, r.context.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.context.job_state.lock().unwrap());
        }
    }
    Ok(())
}