use crusty_core::{
    ParserProcessor,
    CrawlingRules,
    CrawlingRulesOptions,
    Crawler,
    expanders::TaskExpander,
    types::{
        Job, JobStateValues, JobContext, Task, Status as HttpStatus, JobStatus,
        select::predicate::Name, select::document::Document
    },
    config,
};

use anyhow::{Context as _};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct JobState {
    sum_title_len: usize
}
impl JobStateValues for JobState {}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String
}

pub struct DataExtractor {}
impl TaskExpander<JobState, TaskState> for DataExtractor {
    fn expand(
        &self, ctx: &mut JobContext<JobState, TaskState>,
        _task: &Task, _status: &HttpStatus, document: &Document
    ) {
        let title = document.find(Name("title")).next().map(|v|v.text());
        if title.is_some() {
            let title = title.unwrap();
            ctx.task_state.lock().unwrap().title = title.clone();
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let concurrency_profile = config::ConcurrencyProfile::default();
    let (pp, tx_pp) = ParserProcessor::new( concurrency_profile, 1024 * 1024 * 32);
    let h_pp = tokio::spawn(pp.go());

    let settings = config::CrawlerSettings::default();
    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let rules = Box::new(CrawlingRules {
        options: CrawlingRulesOptions::default(),
        ..CrawlingRules::default()}
        .with_task_expanders(|| vec![Box::new(DataExtractor{})] ));

    let crawler = Crawler::new(networking_profile);

    let url = Url::parse("https://bash.im").context("cannot parse url")?;
    let job = Job{url, settings, rules, job_state: JobState::default()};
    for r in crawler.iter(job, tx_pp) {
        println!("- {}, task context: {:?}", r, r.context.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final context: {:?}", r.context.job_state.lock().unwrap());
        }
    }

    Ok(h_pp.await??)
}