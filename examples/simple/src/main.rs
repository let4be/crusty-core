use crusty_core::{
    config,
    select::{document::Document, predicate::Name},
    task_expanders,
    types::{HttpStatus, Job, JobCtx, JobStatus, Task},
    Crawler, CrawlingRules, CrawlingRulesOptions, ParserProcessor, TaskExpander,
};

#[derive(Debug, Clone, Default)]
pub struct JobState {
    sum_title_len: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String,
}

pub struct DataExtractor {}
type Ctx = JobCtx<JobState, TaskState>;
impl TaskExpander<JobState, TaskState> for DataExtractor {
    fn expand(&self, ctx: &mut Ctx, _: &Task, _: &HttpStatus, doc: &Document) -> task_expanders::Result {
        let title = doc.find(Name("title")).next().map(|v| v.text());
        if let Some(title) = title {
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
            ctx.task_state.title = title;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let concurrency_profile = config::ConcurrencyProfile::default();
    let pp = ParserProcessor::spawn(concurrency_profile, 1024 * 1024 * 32);

    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let crawler = Crawler::new(networking_profile, &pp);

    let settings = config::CrawlingSettings::default();
    let rules_opt = CrawlingRulesOptions::default();
    let rules = CrawlingRules::new(rules_opt).with_task_expander(|| DataExtractor {});

    let job = Job::new("https://example.com", settings, rules, JobState::default())?;
    for r in crawler.iter(job) {
        println!("- {}, task state: {:?}", r, r.ctx.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.ctx.job_state.lock().unwrap());
        }
    }

    Ok(pp.join().await??)
}
