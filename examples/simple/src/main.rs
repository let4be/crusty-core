use crusty_core::{
    ParserProcessor, CrawlingRules, CrawlingRulesOptions, Crawler, task_expanders::TaskExpander,
    types::{
        Job, JobContext, Task, Status as HttpStatus, JobStatus,
        select::predicate::Name, select::document::Document
    },
    config,
};

use anyhow::{Context as _};

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
    fn expand(&self,
              ctx: &mut JobContext<JobState, TaskState>,
              _task: &Task, _status: &HttpStatus, doc: &Document
    ) {
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

    let settings = config::CrawlerSettings::default();
    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let rules_options = CrawlingRulesOptions::default();
    let rules = Box::new(CrawlingRules::new(rules_options)
        .with_task_expanders(|| vec![Box::new(DataExtractor{})] ));

    let url = url::Url::parse("https://bash.im").context("cannot parse url")?;
    let job = Job{url, settings, rules, job_state: JobState::default()};
    let crawler = Crawler::new(networking_profile);
    for r in crawler.iter(job, &pp) {
        println!("- {}, task state: {:?}", r, r.context.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.context.job_state.lock().unwrap());
        }
    }

    Ok(pp.join().await??)
}