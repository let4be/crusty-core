use crusty_core::prelude::*;

use std::{collections::HashMap, fmt};
use tracing::{info, Level};
use tracing_subscriber;

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

impl JobState {
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
    fn expand(&self, ctx: &mut JobCtx<JobState, TaskState>, task: &Task, _: &HttpStatus, doc: &Document) {
        let title = doc
            .find(Name("title")).next().map(|v|v.text());
        if let Some(title) = title {
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

async fn process_responses(rx: ChReceiver<JobUpdate<JobState, TaskState>>) {
    while let Ok(r) = rx.recv_async().await {
        info!("- {}, task state: {:?}", r, r.context.task_state);
        if let JobStatus::Finished(_) = r.status {
            let mut ctx = r.context.job_state.lock().unwrap();
            ctx.finalize();
            info!("final job state: {}", ctx);
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
    let pp = ParserProcessor::spawn( concurrency_profile, 1024 * 1024 * 32);

    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let crawler = Crawler::new(networking_profile, &pp);

    let settings = config::CrawlingSettings::default();
    let rules_options = CrawlingRulesOptions{
        page_budget: Some(100),
        link_target: LinkTarget::Follow,
        ..CrawlingRulesOptions::default()
    };
    let rules = CrawlingRules::new(rules_options)
        .with_task_expander(|| DataExtractor{});

    let (update_tx, update_rx) = ch_unbounded();
    let h_sub = tokio::spawn(process_responses(update_rx));

    let job = Job::new("http://192.168.1.1/", settings, rules, JobState::default())?;
    crawler.go(job, update_tx).await?;
    drop(crawler);

    let _ = pp.join().await?;
    let _ = h_sub.await?;
    Ok(())
}