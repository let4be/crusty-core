use humansize::{file_size_opts, FileSize};
use thiserror::{self, Error};

#[allow(unused_imports)]
use crate::_prelude::*;
use crate::{config, load_filters, status_filters, task_expanders, task_filters};

pub trait ParsedDocument: 'static {}
pub type TaskFilters<JS, TS> = Vec<Box<dyn task_filters::Filter<JS, TS> + Send + Sync>>;
pub type StatusFilters<JS, TS> = Vec<Box<dyn status_filters::Filter<JS, TS> + Send + Sync>>;
pub type LoadFilters<JS, TS> = Vec<Box<dyn load_filters::Filter<JS, TS> + Send + Sync>>;
pub type TaskExpanders<JS, TS, P> = Vec<Box<dyn task_expanders::Expander<JS, TS, P> + Send + Sync>>;
pub type DocumentParser<P> = Box<dyn Fn(Box<dyn io::Read + Sync + Send>) -> Result<P> + Send + Sync + 'static>;

pub type BoxedFn<T> = Box<dyn Fn() -> Box<T> + Send + Sync>;
pub type BoxedTaskFilter<JS, TS> = BoxedFn<dyn task_filters::Filter<JS, TS> + Send + Sync>;
pub type BoxedStatusFilter<JS, TS> = BoxedFn<dyn status_filters::Filter<JS, TS> + Send + Sync>;
pub type BoxedLoadFilter<JS, TS> = BoxedFn<dyn load_filters::Filter<JS, TS> + Send + Sync>;
pub type BoxedTaskExpander<JS, TS, P> = BoxedFn<dyn task_expanders::Expander<JS, TS, P> + Send + Sync>;

pub struct Job<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> {
	pub url:       url::Url,
	pub addrs:     Option<Vec<SocketAddr>>,
	pub settings:  Arc<config::CrawlingSettings>,
	pub rules:     Box<dyn JobRules<JS, TS, P>>,
	pub job_state: JS,
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> Job<JS, TS, P> {
	pub fn new<R: JobRules<JS, TS, P>>(
		url: &str,
		settings: config::CrawlingSettings,
		rules: R,
		job_state: JS,
	) -> anyhow::Result<Job<JS, TS, P>> {
		Self::new_with_shared_settings(url, Arc::new(settings), rules, job_state)
	}

	pub fn new_with_shared_settings<R: JobRules<JS, TS, P>>(
		url: &str,
		settings: Arc<config::CrawlingSettings>,
		rules: R,
		job_state: JS,
	) -> anyhow::Result<Job<JS, TS, P>> {
		let url = Url::parse(url).context("cannot parse url")?;

		Ok(Self { url, addrs: None, settings, rules: Box::new(rules), job_state })
	}

	pub fn with_addrs(mut self, addrs: Vec<SocketAddr>) -> Self {
		self.addrs = Some(addrs);
		self
	}
}

#[derive(Derivative)]
#[derivative(Clone(bound = ""))]
pub struct ResolvedJob<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> {
	pub url:      url::Url,
	pub addrs:    Option<Vec<SocketAddr>>,
	pub settings: Arc<config::CrawlingSettings>,
	pub rules:    Arc<Box<dyn JobRules<JS, TS, P>>>,
	pub ctx:      JobCtx<JS, TS>,
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> From<Job<JS, TS, P>> for ResolvedJob<JS, TS, P> {
	fn from(job: Job<JS, TS, P>) -> ResolvedJob<JS, TS, P> {
		ResolvedJob {
			url:      job.url.clone(),
			addrs:    job.addrs,
			settings: Arc::clone(&job.settings),
			rules:    Arc::new(job.rules),
			ctx:      JobCtx::new(job.url, job.settings, job.job_state, TS::default()),
		}
	}
}

pub trait JobRules<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument>: Send + Sync + 'static {
	fn task_filters(&self) -> TaskFilters<JS, TS>;
	fn status_filters(&self) -> StatusFilters<JS, TS>;
	fn load_filters(&self) -> LoadFilters<JS, TS>;
	fn task_expanders(&self) -> TaskExpanders<JS, TS, P>;
	fn document_parser(&self) -> Arc<DocumentParser<P>>;
}

pub type BoxedJobRules<JS, TS, P> = Box<dyn JobRules<JS, TS, P>>;

#[derive(Error, Debug)]
pub enum Error {
	#[error(transparent)]
	Other(#[from] anyhow::Error),
	#[error("timeout while awaiting for status response")]
	StatusTimeout,
	#[error("timeout during loading")]
	LoadTimeout,
}

#[derive(Debug, Clone, IntoStaticStr)]
pub enum FilterKind {
	HeadStatusFilter,
	GetStatusFilter,
	LoadFilter,
	FollowFilter,
}

#[derive(Error, Debug, Clone)]
pub enum ExtStatusError {
	#[error("terminated by {kind:?} {name:?}")]
	Term { kind: FilterKind, name: &'static str, reason: &'static str },
	#[error("error in {kind:?} {name:?}")]
	Err {
		kind:   FilterKind,
		name:   &'static str,
		#[source]
		source: Arc<anyhow::Error>,
	},
	#[error("panic in {kind:?} {name:?}")]
	Panic {
		kind:   FilterKind,
		name:   &'static str,
		#[source]
		source: Arc<anyhow::Error>,
	},
}

pub type Result<T> = anyhow::Result<T, Error>;

#[derive(Error, Debug)]
pub enum ExtError {
	#[error(transparent)]
	Other(#[from] anyhow::Error),
	#[error("terminated because '{reason:?}'")]
	Term { reason: &'static str },
}
pub type ExtResult<T> = std::result::Result<T, ExtError>;

#[derive(Error, Debug)]
pub enum JobError {
	#[error("job finished by soft timeout")]
	JobFinishedBySoftTimeout,
	#[error("job finished by hard timeout")]
	JobFinishedByHardTimeout,
}

#[derive(Clone, PartialEq, Debug, Copy, IntoStaticStr)]
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
	pub rel:             String,
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
pub struct HeaderMap(pub http::HeaderMap<http::HeaderValue>);

impl HeaderMap {
	pub fn get_str(&self, name: http::header::HeaderName) -> anyhow::Result<&str> {
		let name_str = String::from(name.as_str());
		self.0
			.get(name)
			.ok_or_else(|| anyhow!("{}: not found", name_str))?
			.to_str()
			.with_context(|| format!("cannot read {} value", name_str))
	}
}

impl Deref for HeaderMap {
	type Target = http::HeaderMap<http::HeaderValue>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

#[derive(Clone)]
pub struct HttpStatus {
	pub started_at: Instant,
	pub code:       u16,
	pub headers:    HeaderMap,
	pub metrics:    StatusMetrics,
	pub filter_err: Option<ExtStatusError>,
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

pub struct LoadData {
	pub metrics:    LoadMetrics,
	pub filter_err: Option<ExtStatusError>,
}

pub struct StatusResult(pub Option<Result<HttpStatus>>);

impl Deref for StatusResult {
	type Target = Option<Result<HttpStatus>>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub struct LoadResult(pub Option<Result<LoadData>>);

impl Deref for LoadResult {
	type Target = Option<Result<LoadData>>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub struct FollowResult(pub Option<Result<FollowData>>);

impl Deref for FollowResult {
	type Target = Option<Result<FollowData>>;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

pub struct JobProcessing {
	pub resolve_data: ResolveData,
	pub head_status:  StatusResult,
	pub status:       StatusResult,
	pub load:         LoadResult,
	pub follow:       FollowResult,
	pub links:        Vec<Arc<Link>>,
}

impl JobProcessing {
	pub fn new(resolve_data: ResolveData) -> Self {
		Self {
			resolve_data,
			head_status: StatusResult(None),
			status: StatusResult(None),
			load: LoadResult(None),
			follow: FollowResult(None),
			links: vec![],
		}
	}
}

pub struct FollowData {
	pub metrics:    FollowMetrics,
	pub filter_err: Option<ExtStatusError>,
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

pub trait JobStateValues: Send + Sync + 'static {}

impl<T: Send + Sync + 'static> JobStateValues for T {}

pub trait TaskStateValues: Send + Sync + Clone + Default + 'static {}

impl<T: Send + Sync + Clone + Default + 'static> TaskStateValues for T {}

pub type JobSharedState = Arc<Mutex<HashMap<String, Box<dyn std::any::Any + Send + Sync>>>>;

#[derive(Derivative)]
#[derivative(Clone(bound = "TS: Clone"))]
pub struct JobCtx<JS, TS> {
	pub settings:   Arc<config::CrawlingSettings>,
	pub started_at: Instant,
	pub root_url:   Url,
	pub shared:     JobSharedState,
	pub job_state:  Arc<Mutex<JS>>,
	pub task_state: TS,
	links:          Vec<Arc<Link>>,
}

impl<JS: JobStateValues, TS: TaskStateValues> JobCtx<JS, TS> {
	pub fn new(root_url: Url, settings: Arc<config::CrawlingSettings>, job_state: JS, task_state: TS) -> Self {
		Self {
			started_at: Instant::now(),
			root_url,
			settings,
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

	pub fn push_link(&mut self, link: Link) {
		self.links.push(Arc::new(link))
	}

	pub fn push_links<I: IntoIterator<Item = Link>>(&mut self, links: I) {
		self.links.extend(links.into_iter().map(Arc::new))
	}

	pub fn push_shared_links<I: IntoIterator<Item = Arc<Link>>>(&mut self, links: I) {
		self.links.extend(links.into_iter())
	}

	pub(crate) fn consume_links(&mut self) -> Vec<Arc<Link>> {
		let mut links = vec![];
		mem::swap(&mut links, &mut self.links);
		links
	}
}

impl Link {
	pub fn new(
		href: &str,
		rel: &str,
		alt: &str,
		text: &str,
		redirect: usize,
		target: LinkTarget,
		parent: &Link,
	) -> Result<Self> {
		let mut url = Url::parse(href)
			.or_else(|_err| {
				parent.url.join(href).with_context(|| format!("cannot join relative href {} to {}", href, &parent.url))
			})
			.context("cannot parse url")?;
		url.set_fragment(None);

		Ok(Self {
			url,
			rel: String::from(rel),
			alt: alt.trim().to_string(),
			text: text.trim().to_string(),
			redirect,
			target,
			is_waker: false,
		})
	}

	pub(crate) fn new_abs(href: Url, rel: &str, alt: &str, text: &str, redirect: usize, target: LinkTarget) -> Self {
		Self {
			url: href,
			rel: String::from(rel),
			alt: alt.trim().to_string(),
			text: text.trim().to_string(),
			redirect,
			target,
			is_waker: false,
		}
	}
}

impl Task {
	pub(crate) fn new_root(url: &Url) -> Result<Task> {
		let link = Arc::new(Link::new_abs(url.clone(), "", "", "", 0, LinkTarget::Follow));

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
			StatusResult(None) => {
				write!(f, "")
			}
			StatusResult(Some(Ok(ref r))) => {
				write!(
					f,
					"[{}] wait {}ms / status {}ms",
					r.code,
					r.metrics.wait_duration.as_millis(),
					r.metrics.duration.as_millis()
				)
			}
			StatusResult(Some(Err(ref err))) => {
				write!(f, "[err]: {:#}", err)
			}
		}
	}
}

impl fmt::Display for LoadResult {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			LoadResult(Some(Ok(ref r))) => {
				let m = &r.metrics;
				write!(
					f,
					"loaded {}ms / write {} / read {}",
					m.duration.as_millis(),
					m.write_size.file_size(file_size_opts::CONVENTIONAL).unwrap(),
					m.read_size.file_size(file_size_opts::CONVENTIONAL).unwrap()
				)
			}
			LoadResult(Some(Err(ref err))) => {
				write!(f, "[err loading]: {:#}", err)
			}
			LoadResult(None) => {
				write!(f, "none")
			}
		}
	}
}

impl fmt::Display for FollowResult {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			FollowResult(Some(Ok(ref r))) => {
				let m = &r.metrics;
				write!(f, "parsed {}ms", m.duration.as_millis())
			}
			FollowResult(Some(Err(ref err))) => {
				write!(f, "[err following]: {:#}", err)
			}
			FollowResult(None) => {
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
					"{} {} {} (load: {}) | (follow: {}) {}",
					r.resolve_data, r.head_status, r.status, r.load, r.follow, self.task
				)
			}
			JobStatus::Processing(Err(ref err)) => {
				write!(f, "[dns error : {:#}] {}", err, self.task)
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
			write!(f, "{} ({}) {}", &self.link.url, self.level, self.link.text.chars().take(32).collect::<String>())
		}
	}
}
