use robotstxt_with_cache as robotstxt;

#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{types as rt, types::ExtError};

#[derive(Clone, PartialEq, Debug, Copy)]
pub enum Action {
	Accept,
	Skip,
}
pub type Result = rt::ExtResult<Action>;

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
	// filter can have an optional name
	fn name(&self) -> String {
		String::from("no name")
	}
	// filters can be waked when a task with Link.waked is considered finished by a scheduler
	fn wake(&mut self, _ctx: &mut rt::JobCtx<JS, TS>) {}
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
	fn name(&self) -> String {
		self.filter.name()
	}

	fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, task_seq_num: usize, task: &mut rt::Task) -> Result {
		let link_target = self.link_targets.iter().find(|target| target.to_string() == task.link.target.to_string());
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

pub struct MaxRedirect {
	max_redirect: usize,
}

#[derive(Default)]
pub struct HashSetDedup {
	visited: std::collections::HashSet<String>,
}

#[derive(Clone, PartialEq, Debug, Copy)]
enum RobotsTxtState {
	None,
	Requested,
	Decided,
}

pub struct RobotsTxt {
	state:       RobotsTxtState,
	link_buffer: Vec<rt::Link>,
	matcher:     Option<robotstxt::matcher::CachingRobotsMatcher<robotstxt::matcher::LongestMatchRobotsMatchStrategy>>,
}

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
		let mut strip_prefix = String::from("");
		if www_allow {
			strip_prefix = String::from("www.");
		}
		Self { strip_prefix }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for TotalPageBudget {
	name! {}

	fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, _: &mut rt::Task) -> Result {
		if self.budget >= self.allocated_budget {
			return Err(ExtError::Term)
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
		}
		self.links_within_current_task += 1;
		if self.links_within_current_task > self.allocated_budget {
			return Err(ExtError::Term)
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
			return Err(ExtError::Term)
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
		if self.visited.contains(task.link.url.as_str()) {
			return Ok(Action::Skip)
		}

		self.visited.insert(task.link.url.to_string());
		Ok(Action::Accept)
	}
}

impl HashSetDedup {
	struct_name! {}

	pub fn new() -> Self {
		Self { visited: std::collections::HashSet::new() }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for MaxRedirect {
	name! {}

	fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
		if task.link.redirect > self.max_redirect {
			return Ok(Action::Skip)
		}
		Ok(Action::Accept)
	}
}

impl MaxRedirect {
	struct_name! {}

	pub fn new(max_redirect: usize) -> Self {
		Self { max_redirect }
	}
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for RobotsTxt {
	name! {}

	fn wake(&mut self, ctx: &mut rt::JobCtx<JS, TS>) {
		if let Some(robots) = ctx.shared.lock().unwrap().get("robots") {
			if let Some(robots) = robots.downcast_ref::<String>() {
				let mut matcher = robotstxt::DefaultCachingMatcher::new(robotstxt::DefaultMatcher::default());
				matcher.parse(&robots);
				self.matcher = Some(matcher);
			}
		}

		self.state = RobotsTxtState::Decided;

		let mut link_buffer: Vec<rt::Link> = vec![];
		mem::swap(&mut link_buffer, &mut self.link_buffer);
		ctx.push_links(link_buffer);
	}

	fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
		if task.is_root() {
			if self.state != RobotsTxtState::Requested {
				let url = Url::parse(
					format!("{}://{}/robots.txt", ctx.root_url.scheme(), ctx.root_url.host_str().unwrap()).as_str(),
				)
				.context("cannot create robots.txt url")?;

				let mut link = rt::Link::new_abs(url, String::from(""), String::from(""), 0, rt::LinkTarget::Load);
				link.is_waker = true;
				ctx.push_links(vec![link]);

				self.state = RobotsTxtState::Requested;
			}
			return Ok(Action::Accept)
		}

		if self.state == RobotsTxtState::Decided {
			if let Some(ref mut matcher) = &mut self.matcher {
				if !matcher.one_agent_allowed_by_robots(
					ctx.settings.user_agent.as_deref().unwrap_or(""),
					task.link.url.as_str(),
				) {
					trace!("robots.txt resolved(loaded): SKIPPING");
					return Ok(Action::Skip)
				}
				trace!("robots.txt resolved(loaded): ACCEPTING");
				return Ok(Action::Accept)
			}

			trace!("robots.txt resolved(not loaded): ACCEPTING");
			return Ok(Action::Accept)
		}

		if task.link.url.as_str().ends_with("/robots.txt") {
			return Ok(Action::Accept)
		}

		self.link_buffer.push((*task.link).clone());
		Ok(Action::Accept)
	}
}

impl Default for RobotsTxt {
	fn default() -> Self {
		Self { state: RobotsTxtState::None, link_buffer: vec![], matcher: None }
	}
}

impl RobotsTxt {
	struct_name! {}

	pub fn new() -> Self {
		Self::default()
	}
}
