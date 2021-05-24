#[allow(unused_imports)]
use crate::prelude::*;
pub use select;
pub use async_channel;
use crate::{
    task_filters,
    status_filters,
    load_filters,
    task_expanders,
    config
};

use humansize::{file_size_opts, FileSize};
use thiserror::{self, Error};

pub type TaskFilters<JS, TS> = Vec<Box<dyn task_filters::TaskFilter<JS, TS> + Send + Sync>>;
pub type StatusFilters<JS, TS> = Vec<Box<dyn status_filters::StatusFilter<JS, TS> + Send + Sync>>;
pub type LoadFilters<JS, TS> = Vec<Box<dyn load_filters::LoadFilter<JS, TS> + Send + Sync>>;
pub type TaskExpanders<JS, TS> = Vec<Box<dyn task_expanders::TaskExpander<JS, TS> + Send + Sync>>;

pub struct Job<JS: JobStateValues, TS: TaskStateValues> {
    pub url: url::Url,
    pub settings: config::CrawlerSettings,
    pub rules: Box<dyn JobRules<JS, TS>>,
    pub job_state: JS
}

#[derive(Clone)]
pub struct ResolvedJob<JS: JobStateValues, TS: TaskStateValues> {
    pub url: url::Url,
    pub settings: config::CrawlerSettings,
    pub rules: Arc<Box<dyn JobRules<JS, TS>>>,
    pub ctx: JobContext<JS, TS>
}

impl<JS: JobStateValues, TS: TaskStateValues> From<Job<JS, TS>> for ResolvedJob<JS, TS> {
    fn from(job: Job<JS, TS>) -> ResolvedJob<JS, TS> {
        ResolvedJob {
            url: job.url.clone(),
            settings: job.settings,
            rules: Arc::new(job.rules),
            ctx: JobContext::new(job.url, job.job_state, TS::default())
        }
    }
}

pub trait JobRules<JS: JobStateValues, TS: TaskStateValues>: Send + Sync + 'static {
    fn task_filters(&self) -> TaskFilters<JS, TS>;
    fn status_filters(&self) -> StatusFilters<JS, TS>;
    fn load_filters(&self) -> LoadFilters<JS, TS>;
    fn task_expanders(&self) -> TaskExpanders<JS, TS>;
}

pub type BoxedJobRules<JS, TS> = Box<dyn JobRules<JS, TS>>;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error("timeout during loading")]
    LoadTimeout,
    #[error("terminated by filter {name:?}")]
    FilterTerm{
        name: String
    }
}

pub type Result<T> = anyhow::Result<T, Error>;

#[derive(Error, Debug)]
pub enum JobError {
    #[error("job finished by soft timeout")]
    JobFinishedBySoftTimeout,
    #[error("job finished by hard timeout")]
    JobFinishedByHardTimeout,
}

#[derive(Clone, PartialEq, Debug )]
pub enum LinkTarget {
    Head,
    Load,
    HeadLoad,
    Follow,
    HeadFollow,
}

impl fmt::Display for LinkTarget {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub type LinkIter<'a> = Box<dyn Iterator<Item = &'a Link> + Send + 'a>;

#[derive(Clone)]
pub struct Link {
    pub url: Url,
    pub alt: String,
    pub text: String,
    pub redirect: usize,
    pub target: LinkTarget,
}

impl Link {
    pub fn host(&self) -> Option<String> {
        Some(self.url.host()?.to_string().trim().to_lowercase())
    }
}

#[derive(Clone)]
pub struct Status {
    pub started_processing_on: Instant,
    pub status_code: i32,
    pub headers: http::HeaderMap<http::HeaderValue>,
    pub status_metrics: StatusMetrics,
}

#[derive(Clone, Default)]
pub struct StatusMetrics {
    pub wait_time: Duration,
    pub status_time: Duration,
}

#[derive(Clone, Default)]
pub struct LoadMetrics {
    pub load_time: Duration,
    pub read_size: usize,
    pub write_size: usize,
}

#[derive(Clone, Default)]
pub struct FollowMetrics {
    pub parse_time: Duration,
}

pub enum StatusResult {
    None,
    Ok(Status),
    Err(Error)
}

pub enum FollowResult {
    None,
    Ok(FollowData),
    Err(Error)
}

pub enum LoadResult {
    None,
    Ok(LoadData),
    Err(Error)
}

pub struct LoadData {
    pub metrics: LoadMetrics,
}

pub struct StatusData {
    pub head_status: StatusResult,
    pub status: StatusResult,
    pub load_data: LoadResult,
    pub follow_data: FollowResult,
    pub links: Vec<Arc<Link>>
}

pub struct FollowData {
    pub metrics: FollowMetrics,
}

pub struct JobData {
    pub err: Option<JobError>
}

pub enum JobStatus {
    Processing(StatusData),
    Finished(JobData)
}

#[derive(Clone)]
pub struct Task {
    pub queued_at: Instant,
    pub link: Arc<Link>,
    pub level: usize,
}

pub struct JobUpdate<JS: JobStateValues, TS: TaskStateValues> {
    pub task: Arc<Task>,
    pub status: JobStatus,
    pub context: JobContext<JS, TS>
}

pub struct ParserTask {
    pub payload: Box<dyn FnOnce() -> Result<(FollowData, Vec<Arc<Link>>)> + Send + 'static>,
    pub time: Instant,
    pub res_tx: Sender<ParserResponse>
}

pub struct ParserResponse {
    pub res: Result<(FollowData, Vec<Arc<Link>>)>,
    pub wait_time: Duration,
    pub work_time: Duration
}

pub trait JobStateValues: Send + Sync + Clone + 'static {
}

impl<T: Send + Sync + Clone + 'static> JobStateValues for T {}

pub trait TaskStateValues: Send + Sync + Clone + Default + 'static {
}

impl<T: Send + Sync + Clone + Default + 'static> TaskStateValues for T {}

#[derive(Clone)]
pub struct JobContext<JS, TS> {
    pub started_at : Instant,
    pub root_url : Url,
    pub job_state: Arc<Mutex<JS>>,
    pub task_state: Arc<Mutex<TS>>,
    links: Vec<Arc<Link>>,
}

impl<JS: JobStateValues, TS: TaskStateValues> JobContext<JS, TS> {
    pub fn new(root_url: Url, job_state: JS, task_state: TS) -> Self {
        Self {
            started_at: Instant::now(),
            root_url,
            job_state: Arc::new(Mutex::new(job_state)),
            task_state: Arc::new(Mutex::new(task_state)),
            links: vec![],
        }
    }

    pub fn timeout_remaining(&self, t: Duration) -> PinnedFut<()> {
        let elapsed = self.started_at.elapsed();

        Box::pin(async move {
            if t <= elapsed {
                return
            }
            tokio::time::sleep(t - elapsed).await;
        })
    }

    pub fn push_links(&mut self, links: Vec<Link>) {
        self.links.extend(links.into_iter().map(|link|Arc::new(link)))
    }

    pub(crate) fn push_arced_links(&mut self, links: Vec<Arc<Link>>) {
        self.links.extend(links)
    }

    pub fn consume_links(&mut self) -> Vec<Arc<Link>> {
        self.links.drain(0..).collect()
    }
}

impl Link {
    pub(crate) fn new(href: String, alt: String, text: String, redirect: usize, target: LinkTarget, parent: &Link) -> Result<Self> {
        let link_str = href.splitn(2, '#').next().context("cannot prepare link")?;

        let url = Url::parse(link_str).unwrap_or(
            parent.url.join(link_str).with_context(|| format!("cannot join relative href {} to {}", href, parent.url.as_str()))?
        );

        Ok(Self {
            url,
            alt: alt.trim().to_string(),
            text: text.trim().to_string(),
            redirect,
            target
        })
    }

    pub(crate) fn new_abs(href: Url, alt: String, text: String, redirect: usize, target: LinkTarget) -> Self {
        Self {
            url: href,
            alt,
            text,
            redirect,
            target
        }
    }
}

impl Task {
    pub(crate) fn new_root(url: &Url) -> Result<Task> {
        let link = Arc::new(Link::new_abs(url.clone(), "".into(), "".into(), 0, LinkTarget::Follow));

        Ok(Task {
            queued_at: Instant::now(),
            link,
            level: 0,
        })
    }

    pub(crate) fn new(link: Arc<Link>, parent: &Task) -> Result<Task> {
        let scheme = link.url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(anyhow!("invalid scheme {:#?} in {:#?}", scheme, link.url.as_str()).into());
        }

        Ok(Task {
            queued_at: Instant::now(),
            link,
            level: parent.level + 1,
        })
    }

    pub fn is_root(&self) -> bool {
        self.level == 0
    }
}

impl fmt::Display for StatusResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StatusResult::Ok(ref r) => {
                write!(f, "[{}] wait {}ms / status {}ms", r.status_code, r.status_metrics.wait_time.as_millis(), r.status_metrics.status_time.as_millis())
            },
            StatusResult::Err(ref err) => {
                write!(f, "[err]: {}", err.to_string())
            },
            _ => {
                write!(f, "")
            }
        }
    }
}

impl fmt::Display for LoadResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadResult::Ok(ref r) => {
                let m = &r.metrics;
                write!(f, "loaded {}ms / write {} / read {}",
                       m.load_time.as_millis(),
                       m.write_size.file_size(file_size_opts::CONVENTIONAL).unwrap(),
                       m.read_size.file_size(file_size_opts::CONVENTIONAL).unwrap())
            },
            LoadResult::Err(ref err) => {
                write!(f, "[err loading]: {}", err.to_string())
            },
            _ => {
                write!(f, "none")
            }
        }
    }
}

impl fmt::Display for FollowResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FollowResult::Ok(ref r) => {
                let m = &r.metrics;
                write!(f, "parsed {}ms",
                       m.parse_time.as_millis())
            },
            FollowResult::Err(ref err) => {
                write!(f, "[err following]: {}", err.to_string())
            },
            _ => {
                write!(f, "none")
            }
        }
    }
}

impl<JS: JobStateValues, TS: TaskStateValues> fmt::Display for JobUpdate<JS, TS> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            JobStatus::Processing(ref r) => {
                write!(f, "{} {} (load: {}) | (follow: {})", r.head_status, r.status, r.load_data, r.follow_data)
            },
            JobStatus::Finished(ref r) => {
                if r.err.is_some() {
                    write!(f, "[finished : {:?}] {}",
                        r.err.as_ref().unwrap(),
                        self.task
                    )
                } else {
                    write!(f, "[finished] {}",
                        self.task
                    )
                }
            }
        }
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:?}",
               self.url.as_str(),
               self.target,
        )
    }
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.level < 1 {
            write!(f, "{:#?} (root)",
                self.link.url.as_str(),
            )
        } else {
            write!(f, "{:#?} ({}) {:#?}",
                self.link.url.as_str(),
                self.level,
                self.link.text,
            )
        }
    }
}