#[allow(unused_imports)]
use crate::prelude::*;

use std::{
    sync::{Arc, Mutex},
    fmt,
    pin::Pin,
};

use url::Url;
use humansize::{file_size_opts, FileSize};
use thiserror::{self, Error};
use futures::Future;
pub use select;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error("timeout during loading")]
    Timeout,
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
    Load,
    Follow,
}

impl fmt::Display for LinkTarget {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

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
        let host = self.url.host();
        if host.is_none() {
            return None
        }
        Some(host.unwrap().to_string().trim().to_lowercase().to_string())
    }
}

#[derive(Clone)]
pub struct Status {
    pub started_processing_on: Instant,
    pub status_code: i32,
    pub headers: http::HeaderMap<http::HeaderValue>,
    pub status_metrics: StatusMetrics,
    pub links: Vec<Link>,
}

#[derive(Clone)]
pub struct StatusMetrics {
    pub wait_time: Duration,
    pub status_time: Duration,
}

#[derive(Clone)]
pub struct LoadMetrics {
    pub load_time: Duration,
    pub read_size: usize,
    pub write_size: usize,
}

impl Default for StatusMetrics {
    fn default() -> Self {
        Self {
            wait_time: Duration::from_secs(0),
            status_time: Duration::from_secs(0),
        }
    }
}

impl Default for LoadMetrics {
    fn default() -> Self {
        Self {
            load_time: Duration::from_secs(0),
            read_size: 0,
            write_size: 0,
        }
    }
}

#[derive(Clone)]
pub struct FollowMetrics {
    pub parse_time: Duration,
}

impl Default for FollowMetrics {
    fn default() -> Self {
        Self {
            parse_time: Duration::from_secs(0),
        }
    }
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
    pub load_metrics: LoadMetrics,
    pub links: Vec<Link>
}

pub struct StatusData {
    pub status: Status,
    pub load_data: LoadResult,
    pub follow_data: FollowResult
}

pub struct FollowData {
    pub metrics: FollowMetrics,
    pub links: Vec<Link>
}

pub struct JobData {
    pub err: Option<JobError>
}

pub enum JobStatus {
    Processing(Result<StatusData>),
    Finished(JobData)
}

#[derive(Clone)]
pub struct Task {
    pub queued_at: Instant,
    pub link: Link,
    pub parent_link: Option<Link>,
    pub level: usize,
}

pub struct JobUpdate<T: JobContextValues> {
    pub task: Arc<Task>,
    pub status: JobStatus,
    pub context: StdJobContext<T>
}

pub struct ParserTask {
    pub payload: Box<dyn FnOnce() -> Result<FollowData> + Send + 'static>,
    pub time: Instant,
    pub res_tx: Sender<ParserResponse>
}

pub struct ParserResponse {
    pub res: Result<FollowData>,
    pub wait_time: Duration,
    pub work_time: Duration
}

pub trait JobContextValues: Send + Sync + Clone + 'static {
}

#[derive(Clone)]
pub struct StdJobContext<T> {
    pub started_at : Instant,
    pub root_url : Url,
    pub values : Arc<Mutex<T>>,
    links: Vec<Link>
}

impl<T: JobContextValues> StdJobContext<T> {
    pub fn new(root_url: Url, values: T) -> Self {
        Self {
            started_at: Instant::now(),
            root_url,
            values: Arc::new(Mutex::new(values)),
            links: vec![],
        }
    }

    pub fn timeout_remaining(&self, t: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let elapsed = self.started_at.elapsed();

        Box::pin(async move {
            if t <= elapsed {
                return
            }
            tokio::time::sleep(t - elapsed).await;
        })
    }

    pub fn push_links(&mut self, links: Vec<Link>) {
        self.links.extend(links)
    }

    pub fn consume_links(&mut self, ) -> Vec<Link> {
        let links = self.links.clone();
        self.links.clear();
        links
    }
}

impl StatusData {
    pub fn collect_links(&self) -> Box<dyn Iterator<Item = &Link> + Send + '_> {
        let load_links_it: Box<dyn Iterator<Item = &Link> + Send> = if let LoadResult::Ok(load_data) = &self.load_data {
            Box::new(load_data.links.iter())
        } else {
            Box::new(std::iter::empty::<&Link>())
        };
        let follow_links_it: Box<dyn Iterator<Item = &Link> + Send> = if let FollowResult::Ok(follow_data) = &self.follow_data {
            Box::new(follow_data.links.iter())
        } else {
            Box::new(std::iter::empty::<&Link>())
        };
        Box::new(self.status.links.iter().chain(load_links_it).chain(follow_links_it))
    }
}

impl Link {
    pub(crate) fn new(href: String, alt: String, text: String, redirect: usize, target: LinkTarget, parent: &Link) -> Result<Self> {
        let s: Vec<&str> = href.splitn(2, "#").collect();
        let link_str = if s.len() < 1 { href.as_str() } else { s[0] };

        let url = Url::parse(link_str).unwrap_or(
            parent.url.join(link_str).with_context(|| format!("cannot join relative href {} to {}", href, parent.url.as_str()))?
        );

        Ok(Self {
            url,
            alt,
            text,
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
        let link = Link::new_abs(url.clone(), "".into(), "".into(), 0, LinkTarget::Follow);

        Ok(Task {
            queued_at: Instant::now(),
            link,
            parent_link: None::<Link>,
            level: 0,
        })
    }

    pub(crate) fn new(link: Link, parent: &Task) -> Result<Task> {
        let scheme = link.url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(anyhow!("invalid scheme {:#?} in {:#?}", scheme, link.url.as_str()).into());
        }

        Ok(Task {
            queued_at: Instant::now(),
            link,
            parent_link: Some(parent.link.clone()),
            level: parent.level + 1,
        })
    }

    pub fn is_root(&self) -> bool {
        self.level == 0
    }
}

impl<T: JobContextValues> fmt::Display for JobUpdate<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            JobStatus::Processing(ref r) => {
                if r.is_ok() {
                    let ld = r.as_ref().unwrap();
                    let sm = &ld.status.status_metrics;
                    let timings = format!("wait {}ms / status {}ms",
                        sm.wait_time.as_millis(),
                        sm.status_time.as_millis(),
                    );

                    let load_section = match &r.as_ref().unwrap().load_data {
                        LoadResult::None => {
                            format!("none")
                        },
                        LoadResult::Ok(ld) => {
                            let lm = &ld.load_metrics;
                            format!("loaded {}ms / write {} / read {}",
                                    lm.load_time.as_millis(),
                                    lm.write_size.file_size(file_size_opts::CONVENTIONAL).unwrap(),
                                    lm.read_size.file_size(file_size_opts::CONVENTIONAL).unwrap(),
                            )
                        },
                        LoadResult::Err(err) => {
                            format!("[err] loading {}", err.to_string())
                        }
                    };

                    let (header, follow_section) = match &r.as_ref().unwrap().follow_data {
                        FollowResult::None => {
                            (format!("[{}] loaded {}", ld.status.status_code, self.task), format!(""))
                        },
                        FollowResult::Ok(fd) => {
                            (format!("[{}] followed {}", ld.status.status_code, self.task), format!("parsed {}ms", fd.metrics.parse_time.as_millis()))
                        },
                        FollowResult::Err(err) => {
                            (format!("[{}] followed with err {}: {}", ld.status.status_code, self.task, err.to_string()), format!(""))
                        }
                    };

                    write!(f, "{} ({}). (load: {}) | (follow: {})", header, timings, load_section, follow_section)
                } else {
                    write!(f, "[err] loading {} {}",
                        r.as_ref().err().unwrap().to_string(),
                        self.task
                    )
                }
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