use robotstxt_with_cache as robotstxt;

use crate::{_prelude::*, types as rt, types::ExtError};

#[derive(Clone, PartialEq, Debug, Copy)]
pub enum Action {
	Accept,
	Skip,
}
pub type Result = rt::ExtResult<Action>;
pub static TOTAL_PAGE_BUDGET_TERM_REASON: &str = "TotalPageBudget";
pub static LINK_PER_PAGE_BUDGET_TERM_REASON: &str = "LinkPerPageBudget";
pub static MAX_LEVEL_TERM_REASON: &str = "MaxLevel";

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
	// filter can have an optional name
	fn name(&self) -> &'static str {
		"no name"
	}
	// the job of a filter is to determine what to do with a certain task - accept/skip
	fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, task_seq_num: usize, task: &mut rt::Task) -> Result;
}

pub struct SelectiveTaskFilter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
	link_targets: Vec<rt::LinkTarget>,
	filter:       Box<dyn Filter<JS, TS> + Send + Sync>,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> SelectiveTaskFilter<JS, TS> {
	pub fn new(link_targets: Vec<rt::LinkTarget>, filter: impl Filter<JS, TS> + Send + Sync + 'static) -> Self {
		Self { link_targets, filter: Box::new(filter) }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for SelectiveTaskFilter<JS, TS> {
	fn name(&self) -> &'static str {
		self.filter.name()
	}

	fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, task_seq_num: usize, task: &mut rt::Task) -> Result {
		let link_target = self.link_targets.iter().find(|target| **target == task.link.target);
		if link_target.is_none() {
			return Ok(Action::Accept)
		}
		self.filter.accept(ctx, task_seq_num, task)
	}
}

pub struct SameDomain {
	strip_prefix: String,
}

pub struct TotalPageBudget {
	allocated_budget: usize,
	budget:           usize,
}

pub struct LinkPerPageBudget {
	current_task_seq_num:      usize,
	links_within_current_task: usize,
	allocated_budget:          usize,
}

pub struct PageLevel {
	max_level: usize,
}

#[derive(Default, Clone)]
pub struct HashSetDedup {
	visited:     Arc<Mutex<std::collections::HashSet<String>>>,
	is_checking: bool,
}

#[derive(Derivative)]
#[derivative(Default)]
#[derive(Clone, PartialEq, Debug, Copy)]
enum RobotsTxtState {
	#[derivative(Default)]
	None,
	Requested,
	FilteringEnabled,
}

#[derive(Default)]
pub struct RobotsTxt {
	state:   RobotsTxtState,
	matcher: Option<robotstxt::matcher::CachingRobotsMatcher<robotstxt::matcher::LongestMatchRobotsMatchStrategy>>,
}

#[derive(Default)]
pub struct SkipNoFollowLinks {}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for SameDomain {
	name! {}

	fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
		let domain = task.link.url.host_str().unwrap_or("_");

		let root_url = &ctx.root_url;
		let root_domain = root_url.host_str().unwrap_or("");
		if root_domain.strip_prefix(&self.strip_prefix).unwrap_or(root_domain)
			== domain.strip_prefix(&self.strip_prefix).unwrap_or(domain)
		{
			return Ok(Action::Accept)
		}
		Ok(Action::Skip)
	}
}

impl SameDomain {
	struct_name! {}

	pub fn new(www_allow: bool) -> Self {
		Self { strip_prefix: String::from(if www_allow { "www." } else { "" }) }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for TotalPageBudget {
	name! {}

	fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, _: &mut rt::Task) -> Result {
		if self.budget >= self.allocated_budget {
			return Err(ExtError::Term { reason: TOTAL_PAGE_BUDGET_TERM_REASON })
		}
		self.budget += 1;
		Ok(Action::Accept)
	}
}

impl TotalPageBudget {
	struct_name! {}

	pub fn new(allocated_budget: usize) -> Self {
		Self { allocated_budget, budget: 0 }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for LinkPerPageBudget {
	name! {}

	fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, seq_num: usize, _: &mut rt::Task) -> Result {
		if seq_num > self.current_task_seq_num {
			self.links_within_current_task = 0;
			self.current_task_seq_num = seq_num;
		}
		self.links_within_current_task += 1;
		if self.links_within_current_task > self.allocated_budget {
			return Err(ExtError::Term { reason: LINK_PER_PAGE_BUDGET_TERM_REASON })
		}
		Ok(Action::Accept)
	}
}

impl LinkPerPageBudget {
	struct_name! {}

	pub fn new(allocated_budget: usize) -> Self {
		Self { allocated_budget, current_task_seq_num: 0, links_within_current_task: 0 }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for PageLevel {
	name! {}

	fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
		if task.level >= self.max_level {
			return Err(ExtError::Term { reason: MAX_LEVEL_TERM_REASON })
		}
		Ok(Action::Accept)
	}
}

impl PageLevel {
	struct_name! {}

	pub fn new(max_level: usize) -> Self {
		Self { max_level }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for HashSetDedup {
	name! {}

	fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
		let mut visited = self.visited.lock().unwrap();

		if self.is_checking {
			if visited.contains(task.link.url.as_str()) {
				return Ok(Action::Skip)
			}
			return Ok(Action::Accept)
		}

		visited.insert(task.link.url.to_string());
		Ok(Action::Accept)
	}
}

impl HashSetDedup {
	struct_name! {}

	pub fn new(is_checking: bool) -> Self {
		Self { visited: Arc::new(Mutex::new(std::collections::HashSet::new())), is_checking }
	}

	pub fn committing(&self) -> Self {
		let mut filter = self.clone();
		filter.is_checking = false;
		filter
	}
}

pub const ROBOTS_TXT_LINK_MARKER: u8 = 1;

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for RobotsTxt {
	name! {}

	fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, _n: usize, task: &mut rt::Task) -> Result {
		match self.state {
			RobotsTxtState::None => {
				if !task.is_root() {
					return Err(anyhow!("RobotsTxtState = None but we got non root task!").into())
				}

				let url = Url::parse(
					format!(
						"{}://{}/robots.txt",
						ctx.root_url.scheme(),
						ctx.root_url.host_str().context("missing host")?
					)
					.as_str(),
				)
				.context("cannot create robots.txt url")?;

				let mut link = rt::Link::new_abs(url, "", "", "", 0, rt::LinkTarget::Load);
				link.marker = ROBOTS_TXT_LINK_MARKER;

				let mut link = Arc::new(link);
				mem::swap(&mut link, &mut task.link);

				let mut link = (*link).clone();
				// treat this link "as a redirect" from robots.txt, this way it retains root task status
				link.redirect = 1;

				ctx.shared.lock().unwrap().insert(String::from("robots::root_link"), Box::new(Some(link)));

				self.state = RobotsTxtState::Requested;
				Ok(Action::Accept)
			}
			RobotsTxtState::Requested => {
				if !task.is_root() {
					return Err(anyhow!("RobotsTxtState = Requested but we got non root task!").into())
				}

				// accept robots.txt itself
				if task.link.marker == ROBOTS_TXT_LINK_MARKER {
					return Ok(Action::Accept)
				}

				// by default filtering is enabled
				// if we failed to load robots.txt due to some error - we would not have ended here in the first place
				// as load_filter would not emit root_link
				self.state = RobotsTxtState::FilteringEnabled;

				if let Some(robots) = ctx.shared.lock().unwrap().get_mut("robots::matcher") {
					if let Some(ref mut matcher) = robots.downcast_mut::<Option<robotstxt::DefaultCachingMatcher>>() {
						self.matcher = matcher.take();
					}
				}
				self.accept(ctx, _n, task)
			}
			RobotsTxtState::FilteringEnabled => {
				if let Some(ref mut matcher) = &mut self.matcher {
					let user_agent = ctx.settings.user_agent.as_deref().unwrap_or("");
					if matcher.one_agent_allowed_by_robots(user_agent, task.link.url.as_str()) {
						return Ok(Action::Accept)
					}
					return Ok(Action::Skip)
				}

				// looks like there was some error during filter construction
				// whatever it is - we should skip the website UNLESS we got 4xx code(handled by FilteringSkipped state)
				Ok(Action::Skip)
			}
		}
	}
}

impl RobotsTxt {
	struct_name! {}

	pub fn new() -> Self {
		Self::default()
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for SkipNoFollowLinks {
	name! {}

	fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
		if task.link.rel.to_lowercase() == "no-follow" {
			return Ok(Action::Skip)
		}
		Ok(Action::Accept)
	}
}

impl SkipNoFollowLinks {
	struct_name! {}

	pub fn new() -> Self {
		Self {}
	}
}
