use std::{
    collections::{hash_map::Entry, HashMap},
    env, fmt,
    sync::Arc,
};

use anyhow::anyhow;
use crusty_core::{prelude::*, task_expanders::FollowLinks};
use tracing::{info, Level};

type Result<T> = anyhow::Result<T>;

#[derive(Debug, Default)]
pub struct JobState {
    duplicate_titles: HashMap<String, Vec<url::Url>>,
}
impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self
            .duplicate_titles
            .iter()
            .map(|(k, u)| format!("'{}': [{}]", k, u.iter().map(|u| u.to_string()).collect::<Vec<String>>().join(", ")))
            .collect::<Vec<String>>()
            .join("; ");
        write!(f, "{}", r)
    }
}

impl JobState {
    fn finalize(&mut self) {
        self.duplicate_titles = self
            .duplicate_titles
            .iter()
            .filter_map(|(k, v)| if v.len() < 2 { None } else { Some((k.clone(), v.clone())) })
            .collect();
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String,
}
pub struct DataExtractor {}
impl TaskExpander<JobState, TaskState, SelectDocument> for DataExtractor {
    fn name(&self) -> String {
        String::from("My Fancy Data Extractor With Fancy Name")
    }

    fn expand(
        &self,
        ctx: &mut JobCtx<JobState, TaskState>,
        task: &Task,
        _: &HttpStatus,
        doc: &SelectDocument,
    ) -> task_expanders::Result {
        let title = doc.find(Name("title")).next().map(|v| v.text()).ok_or_else(|| anyhow!("title not found"))?;
        ctx.task_state.title = title.clone();

        {
            let mut job_state = ctx.job_state.lock().unwrap();
            let urls_with_title = match job_state.duplicate_titles.entry(title) {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => e.insert(vec![]),
            };
            urls_with_title.push(task.link.url.clone());
        }
        Ok(())
    }
}

async fn process_responses(rx: ChReceiver<JobUpdate<JobState, TaskState>>) {
    while let Ok(r) = rx.recv_async().await {
        info!("- {}, task state: {:?}", r, r.ctx.task_state);
        if let JobStatus::Finished(_) = r.status {
            let mut ctx = r.ctx.job_state.lock().unwrap();
            ctx.finalize();
            info!("final job state: {}", ctx);
        }
    }
}

fn configure_tracing() -> Result<()> {
    let collector = tracing_subscriber::fmt().with_target(false).with_max_level(Level::INFO).finish();
    tracing::subscriber::set_global_default(collector)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    configure_tracing()?;

    let job_url = if let Ok(job_url) = env::var("JOB_URL") {
        job_url
    } else {
        return Err(anyhow!("please specify JOB_URL env. variable"))
    };

    let concurrency_profile =
        config::ConcurrencyProfile { parser_concurrency: 2, ..config::ConcurrencyProfile::default() };
    let tx_pp = ParserProcessor::spawn(concurrency_profile, 1024 * 1024 * 32);

    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let crawler = Crawler::new(networking_profile, tx_pp);

    let settings = config::CrawlingSettings::default();
    let rules_options = CrawlingRulesOptions { page_budget: Some(100), ..CrawlingRulesOptions::default() };
    let rules = CrawlingRules::new(rules_options, select_document_parser())
        .with_task_expander(|| DataExtractor {})
        .with_task_expander(|| FollowLinks::new(LinkTarget::HeadFollow));

    let (update_tx, update_rx) = ch_unbounded();
    let h_sub = tokio::spawn(process_responses(update_rx));

    let job = Job::new(&job_url, Arc::new(settings), rules, JobState::default())?;
    crawler.go(job, update_tx).await?;
    drop(crawler);

    let _ = h_sub.await?;
    Ok(())
}
