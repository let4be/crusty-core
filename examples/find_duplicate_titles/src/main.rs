use crusty_core::{
    ParserProcessor,
    CrawlingRules,
    CrawlingRulesOptions,
    Crawler,
    expanders::TaskExpander,
    types::async_channel::Receiver,
    types,
    config,
};

use std::{collections::HashMap, fmt};
use tracing::{info, Level};
use tracing_subscriber;
use anyhow::{Context as _};
use url::Url;

type Result<T> = anyhow::Result<T>;

#[derive(Debug, Clone, Default)]
pub struct JobState {
    duplicate_titles: HashMap<String, Vec<url::Url>>
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.duplicate_titles.iter().map(|(k, u)| {
            format!("'{}': [{}]", k, u.iter().map(|u|u.to_string()).collect::<Vec<String>>().join(", "))
        }).collect::<Vec<String>>().join("; ");
        write!(f, "{}", r)
    }
}

impl types::JobStateValues for JobState {
    fn finalize(&mut self) {
        self.duplicate_titles = self.duplicate_titles.iter().filter_map(|(k, v)|{
            if v.len() < 2 { None } else { Some((k.clone(), v.clone())) }
        }).collect();
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String
}

pub struct DataExtractor {}
impl TaskExpander<JobState, TaskState> for DataExtractor {
    fn expand(&self,
              ctx: &mut types::StdJobContext<JobState, TaskState>,
              task: &types::Task,
              _status: &types::Status,
              document: &types::select::document::Document)
    {
        let title = document
            .find(types::select::predicate::Name("title")).next().map(|v|v.text());
        if title.is_some() {
            let title = title.unwrap();
            ctx.task_state.lock().unwrap().title = title.clone();

            {
                let mut job_state = ctx.job_state.lock().unwrap();
                let mut urls_with_title = job_state.duplicate_titles.get_mut(&title);
                if urls_with_title.is_none() {
                    job_state.duplicate_titles.insert(title.clone(), vec![]);
                    urls_with_title = job_state.duplicate_titles.get_mut(&title);
                }
                urls_with_title.unwrap().push(task.link.url.clone());
            }
        }
    }
}

async fn process_responses(rx: Receiver<types::JobUpdate<JobState, TaskState>>) {
    while let Ok(r) = rx.recv().await {
        info!("- {}, task context: {:?}", r, r.context.task_state);
        if let types::JobStatus::Finished(_) = r.status {
            info!("final context: {}", r.context.job_state.lock().unwrap());
        }
    }
}

fn configure_tracing() -> Result<()>{
    let collector = tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(collector)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    configure_tracing()?;

    let concurrency_profile = config::ConcurrencyProfile{
        parser_concurrency: 2,
        ..config::ConcurrencyProfile::default()
    };
    let (pp, tx_pp) = ParserProcessor::new( concurrency_profile, 1024 * 1024 * 32);
    let h_pp = tokio::spawn(pp.go());

    let crawler_settings = config::CrawlerSettings::default();
    let networking_profile = config::NetworkingProfile::default();
    let rules = CrawlingRules {
        options: CrawlingRulesOptions{
            page_budget: Some(100),
            ..CrawlingRulesOptions::default()
        },
        ..CrawlingRules::default()}
        .with_custom_task_expanders(|| vec![Box::new(DataExtractor{})] );

    let crawler = Crawler::<JobState, TaskState, _, _>::new(crawler_settings, networking_profile, rules);

    let (update_tx, update_rx) = types::async_channel::unbounded();

    let h_sub = tokio::spawn(process_responses(update_rx));

    let url = Url::parse("https://bash.im").context("cannot parse url")?;
    crawler.go(url, tx_pp, update_tx)?.await?;

    let _ = tokio::join!(h_pp, h_sub);
    Ok(())
}