use crusty_core::{
    ParserProcessor, CrawlingRules, CrawlingRulesOptions, Crawler, TaskExpander,
    types::{
        Job, JobContext as JobCtx, Task, Status as HttpStatus, JobStatus,
        select::predicate::Name, select::document::Document
    },
    config,
};

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
    let concurrency_profile = config::ConcurrencyProfile::default();
    let pp = ParserProcessor::spawn(concurrency_profile, 1024 * 1024 * 32);

    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let crawler = Crawler::new(networking_profile, &pp);

    let settings = config::CrawlerSettings::default();
    let rules_opt = CrawlingRulesOptions::default();
    let rules = CrawlingRules::new(rules_opt).with_task_expander(|| DataExtractor{} );

    let job = Job::new("https://bash.im", settings, rules, JobState::default())?;
    for r in crawler.iter(job) {
        println!("- {}, task state: {:?}", r, r.context.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.context.job_state.lock().unwrap());
        }
    }

    Ok(pp.join().await??)
}