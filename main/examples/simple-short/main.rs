use crusty_core::{prelude::*, select_task_expanders::FollowLinks};

#[derive(Debug, Default)]
pub struct JobState {
    sum_title_len: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String,
}

pub struct DataExtractor {}
type Ctx = JobCtx<JobState, TaskState>;
impl TaskExpander<JobState, TaskState, Document> for DataExtractor {
    fn expand(
        &self,
        ctx: &mut Ctx,
        _: &Task,
        _: &HttpStatus,
        doc: &Document,
    ) -> task_expanders::Result {
        if let Some(title) = doc.find(Name("title")).next().map(|v| v.text()) {
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
            ctx.task_state.title = title;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let crawler = Crawler::new_default()?;

    let settings = config::CrawlingSettings::default();
    let rules = CrawlingRules::new(CrawlingRulesOptions::default(), document_parser())
        .with_task_expander(|| DataExtractor {})
        .with_task_expander(|| FollowLinks::new(LinkTarget::HeadFollow));

    let job = Job::new("https://example.com", settings, rules, JobState::default())?;
    for r in crawler.iter(job) {
        println!("- {}, task state: {:?}", r, r.ctx.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.ctx.job_state.lock().unwrap());
        }
    }
    Ok(())
}
