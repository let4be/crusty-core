use humansize::{file_size_opts, FileSize};
use thiserror::{self, Error};

#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{config, load_filters, status_filters, task_expanders, task_filters};

pub type TaskFilters<JS, TS> = Vec<Box<dyn task_filters::Filter<JS, TS> + Send + Sync>>;
pub type StatusFilters<JS, TS> = Vec<Box<dyn status_filters::Filter<JS, TS> + Send + Sync>>;
pub type LoadFilters<JS, TS> = Vec<Box<dyn load_filters::Filter<JS, TS> + Send + Sync>>;
pub type TaskExpanders<JS, TS> = Vec<Box<dyn task_expanders::Expander<JS, TS> + Send + Sync>>;

pub type BoxedFn<T> = Box<dyn Fn() -> Box<T> + Send + Sync>;
pub type BoxedTaskFilter<JS, TS> = BoxedFn<dyn task_filters::Filter<JS, TS> + Send + Sync>;
pub type BoxedStatusFilter<JS, TS> = BoxedFn<dyn status_filters::Filter<JS, TS> + Send + Sync>;
pub type BoxedLoadFilter<JS, TS> = BoxedFn<dyn load_filters::Filter<JS, TS> + Send + Sync>;
pub type BoxedTaskExpander<JS, TS> = BoxedFn<dyn task_expanders::Expander<JS, TS> + Send + Sync>;

pub struct Job<JS: JobStateValues, TS: TaskStateValues> {
	pub url:       url::Url,
	pub settings:  config::CrawlingSettings,
	pub rules:     Box<dyn JobRules<JS, TS>>,
	pub job_state: JS,
}

impl<JS: JobStateValues, TS: TaskStateValues> Job<JS, TS> {
	pub fn new<R: JobRules<JS, TS>>(
		url: &str,
		settings: config::CrawlingSettings,
		rules: R,
		job_state: JS,
	) -> anyhow::Result<Job<JS, TS>> {
		let url = Url::parse(url).context("cannot parse url")?;

		Ok(Self { url, settings, rules: Box::new(rules), job_state })
	}
}

#[derive(Clone)]
pub struct ResolvedJob<JS: JobStateValues, TS: TaskStateValues> {
	pub url:      url::Url,
	pub settings: config::CrawlingSettings,
	pub rules:    Arc<Box<dyn JobRules<JS, TS>>>,
	pub ctx:      JobCtx<JS, TS>,
}

impl<JS: JobStateValues, TS: TaskStateValues> From<Job<JS, TS>> for ResolvedJob<JS, TS> {
	fn from(job: Job<JS, TS>) -> ResolvedJob<JS, TS> {
		let mut r = ResolvedJob {
			url:      job.url.clone(),
			settings: job.settings,
			rules:    Arc::new(job.rules),
			ctx:      JobCtx::new(job.url, job.job_state, TS::default()),
		};
		r.settings.build();
		r
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
	#[error("terminated by status filter {name:?}")]
	StatusFilterTerm { name: String },
	#[error("terminated by error in status filter(term_by_error=true) {name:?}")]
	StatusFilterTermByError {
		name:   String,
		#[source]
		source: anyhow::Error,
	},
	#[error("terminated by load filter {name:?}")]
	LoadFilterTerm { name: String },
	#[error("terminated by error in load filter(term_by_error=true) {name:?}")]
	LoadFilterTermByError {
		name:   String,
		#[source]
		source: anyhow::Error,
	},
	#[error("terminated by task expander {name:?}")]
	TaskExpanderTerm { name: String },
	#[error("terminated by error in task expander(term_by_error=true) {name:?}")]
	TaskExpanderTermByError {
		name:   String,
		#[source]
		source: anyhow::Error,
	},
}

pub type Result<T> = anyhow::Result<T, Error>;

#[derive(Error, Debug)]
pub enum ExtError {
	#[error(transparent)]
	Other(#[from] anyhow::Error),
	#[error("terminated")]
	Term,
}
pub type ExtResult<T> = std::result::Result<T, ExtError>;

#[derive(Error, Debug)]
pub enum JobError {
	#[error("job finished by soft timeout")]
	JobFinishedBySoftTimeout,
	#[error("job finished by hard timeout")]
	JobFinishedByHardTimeout,
}

#[derive(Clone, PartialEq, Debug, Copy)]
pub enum LinkTarget {
	JustResolveDNS = 0,
	Head           = 1,
	Load           = 2,
	HeadLoad       = 3,
	Follow         = 4,
	HeadFollow     = 5,
}

impl fmt::Display for LinkTarget {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{:?}", self)
	}
}

pub type LinkIter<'a> = Box<dyn Iterator<Item = &'a Link> + Send + 'a>;

#[derive(Clone)]
pub struct Link {
	pub url:             Url,
	pub alt:             String,
	pub text:            String,
	pub redirect:        usize,
	pub target:          LinkTarget,
	pub(crate) is_waker: bool,
}

impl Link {
	pub fn host(&self) -> Option<String> {
		Some(self.url.host()?.to_string().trim().to_lowercase())
	}
}

#[derive(Clone, Default)]
pub struct ResolveMetrics {
	pub duration: Duration,
}

#[derive(Clone)]
pub struct ResolveData {
	pub addrs:   Vec<SocketAddr>,
	pub metrics: ResolveMetrics,
}

#[derive(Clone)]
pub struct HttpStatus {
	pub started_at: Instant,
	pub code:       i32,
	pub headers:    http::HeaderMap<http::HeaderValue>,
	pub metrics:    StatusMetrics,
}

#[derive(Clone, Default)]
pub struct StatusMetrics {
	pub wait_duration: Duration,
	pub duration:      Duration,
}

#[derive(Clone, Default)]
pub struct LoadMetrics {
	pub wait_duration: Duration,
	pub duration:      Duration,
	pub read_size:     usize,
	pub write_size:    usize,
}

#[derive(Clone, Default)]
pub struct FollowMetrics {
	pub duration: Duration,
}

pub enum StatusResult {
	None,
	Ok(HttpStatus),
	Err(Error),
}

pub enum FollowResult {
	None,
	Ok(FollowData),
	Err(Error),
}

pub enum LoadResult {
	None,
	Ok(LoadData),
	Err(Error),
}

pub struct LoadData {
	pub metrics: LoadMetrics,
}

pub struct JobProcessing {
	pub resolve_data: ResolveData,
	pub head_status:  StatusResult,
	pub status:       StatusResult,
	pub load:         LoadResult,
	pub follow:       FollowResult,
	pub links:        Vec<Arc<Link>>,
}

pub struct FollowData {
	pub metrics: FollowMetrics,
}

pub struct JobFinished {}

pub enum JobStatus {
	Processing(Result<JobProcessing>),
	Finished(std::result::Result<JobFinished, JobError>),
}

#[derive(Clone)]
pub struct Task {
	pub queued_at: Instant,
	pub link:      Arc<Link>,
	pub level:     usize,
}

pub struct JobUpdate<JS: JobStateValues, TS: TaskStateValues> {
	pub task:   Arc<Task>,
	pub status: JobStatus,
	pub ctx:    JobCtx<JS, TS>,
}

pub struct ParserTask {
	pub payload: Box<dyn FnOnce() -> Result<FollowData> + Send + 'static>,
	pub time:    Instant,
	pub res_tx:  Sender<ParserResponse>,
}

pub struct ParserResponse {
	pub payload:       Result<FollowData>,
	pub wait_duration: Duration,
	pub work_duration: Duration,
}

pub trait JobStateValues: Send + Sync + Clone + 'static {}

impl<T: Send + Sync + Clone + 'static> JobStateValues for T {}

pub trait TaskStateValues: Send + Sync + Clone + Default + 'static {}

impl<T: Send + Sync + Clone + Default + 'static> TaskStateValues for T {}

pub type JobSharedState = Arc<Mutex<HashMap<String, Box<dyn std::any::Any + Send + Sync>>>>;

#[derive(Clone)]
pub struct JobCtx<JS, TS> {
	pub started_at: Instant,
	pub root_url:   Url,
	pub shared:     JobSharedState,
	pub job_state:  Arc<Mutex<JS>>,
	pub task_state: TS,
	links:          Vec<Link>,
}

impl<JS: JobStateValues, TS: TaskStateValues> JobCtx<JS, TS> {
	pub fn new(root_url: Url, job_state: JS, task_state: TS) -> Self {
		Self {
			started_at: Instant::now(),
			root_url,
			shared: Arc::new(Mutex::new(HashMap::new())),
			job_state: Arc::new(Mutex::new(job_state)),
			task_state,
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
		self.links.extend(links.into_iter())
	}

	pub fn consume_links(&mut self) -> Vec<Arc<Link>> {
		self.links.drain(0..).map(Arc::new).collect()
	}

	pub(crate) fn _consume_links(&mut self) -> Vec<Link> {
		self.links.drain(0..).collect()
	}
}

impl Link {
	pub(crate) fn new(
		href: String,
		alt: String,
		text: String,
		redirect: usize,
		target: LinkTarget,
		parent: &Link,
	) -> Result<Self> {
		let link_str = href.splitn(2, '#').next().context("cannot prepare link")?;

		let url = Url::parse(link_str).unwrap_or(
			parent
				.url
				.join(link_str)
				.with_context(|| format!("cannot join relative href {} to {}", href, &parent.url))?,
		);

		Ok(Self { url, alt: alt.trim().to_string(), text: text.trim().to_string(), redirect, target, is_waker: false })
	}

	pub(crate) fn new_abs(href: Url, alt: String, text: String, redirect: usize, target: LinkTarget) -> Self {
		Self { url: href, alt, text, redirect, target, is_waker: false }
	}
}

impl Task {
	pub(crate) fn new_root(url: &Url) -> Result<Task> {
		let link = Arc::new(Link::new_abs(url.clone(), "".into(), "".into(), 0, LinkTarget::Follow));

		Ok(Task { queued_at: Instant::now(), link, level: 0 })
	}

	pub(crate) fn new(link: Arc<Link>, parent: &Task) -> Result<Task> {
		let scheme = link.url.scheme();
		if scheme != "http" && scheme != "https" {
			return Err(anyhow!("invalid scheme {:#?} in {:#?}", scheme, link.url.as_str()).into())
		}

		Ok(Task { queued_at: Instant::now(), link, level: parent.level + 1 })
	}

	pub fn is_root(&self) -> bool {
		self.level == 0
	}
}

impl fmt::Display for ResolveData {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let addrs_str = self.addrs.iter().map(|a| a.ip().to_string()).collect::<Vec<String>>().join(", ");
		write!(f, "[{}] resolve {}ms", addrs_str, self.metrics.duration.as_millis())
	}
}

impl fmt::Display for StatusResult {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			StatusResult::Ok(ref r) => {
				write!(
					f,
					"[{}] wait {}ms / status {}ms",
					r.code,
					r.metrics.wait_duration.as_millis(),
					r.metrics.duration.as_millis()
				)
			}
			StatusResult::Err(ref err) => {
				write!(f, "[err]: {}", err.to_string())
			}
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
				write!(
					f,
					"loaded {}ms / write {} / read {}",
					m.duration.as_millis(),
					m.write_size.file_size(file_size_opts::CONVENTIONAL).unwrap(),
					m.read_size.file_size(file_size_opts::CONVENTIONAL).unwrap()
				)
			}
			LoadResult::Err(ref err) => {
				write!(f, "[err loading]: {}", err.to_string())
			}
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
				write!(f, "parsed {}ms", m.duration.as_millis())
			}
			FollowResult::Err(ref err) => {
				write!(f, "[err following]: {}", err.to_string())
			}
			_ => {
				write!(f, "none")
			}
		}
	}
}

impl<JS: JobStateValues, TS: TaskStateValues> fmt::Display for JobUpdate<JS, TS> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.status {
			JobStatus::Processing(Ok(ref r)) => {
				write!(
					f,
					"{} {} {} (load: {}) | (follow: {})",
					r.resolve_data, r.head_status, r.status, r.load, r.follow
				)
			}
			JobStatus::Processing(Err(ref err)) => {
				write!(f, "[dns error : {:?}] {}", err, self.task)
			}
			JobStatus::Finished(Ok(ref r)) => {
				write!(f, "[finished] {} {}", self.task, r)
			}
			JobStatus::Finished(Err(ref err)) => {
				write!(f, "[finished : {:?}] {}", err, self.task)
			}
		}
	}
}

impl fmt::Display for Link {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} {:?}", &self.url, self.target)
	}
}

impl fmt::Display for JobFinished {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "")
	}
}

impl fmt::Display for Task {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		if self.level < 1 {
			write!(f, "{} (root)", &self.link.url)
		} else {
			write!(f, "{} ({}) {}", &self.link.url, self.level, self.link.text)
		}
	}
}
