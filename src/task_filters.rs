#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::types as rt;
use crate::types::ExtError;

#[derive(Clone, PartialEq, Debug, Copy )]
pub enum Action {
    Accept,
    Skip,
}
pub type ExtResult = rt::ExtResult<Action>;

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> String;
    fn accept(
        &mut self,
        ctx: &mut rt::JobCtx<JS, TS>,
        task_seq_num: usize,
        task: &mut rt::Task,
    ) -> ExtResult;
}

pub struct SelectiveTaskFilter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    link_targets: Vec<rt::LinkTarget>,
    filter: Box<dyn Filter<JS, TS> + Send + Sync>,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> SelectiveTaskFilter<JS, TS> {
    pub fn new(link_targets: Vec<rt::LinkTarget>, filter: impl Filter<JS, TS> + Send + Sync + 'static) -> Self {
        Self {
            link_targets, filter: Box::new(filter)
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for SelectiveTaskFilter<JS, TS> {
    fn name(&self) -> String {
        self.filter.name()
    }
    fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, task_seq_num: usize, task: &mut rt::Task) -> ExtResult {
        let link_target = self.link_targets.iter().find(|target| target.to_string() == task.link.target.to_string());
        if link_target.is_none() {
            return Ok(Action::Accept)
        }
        self.filter.accept( ctx, task_seq_num, task)
    }
}

pub struct SameDomain {
    strip_prefix: String,
}

pub struct TotalPageBudget {
    allocated_budget: usize,
    budget: usize,
}

pub struct LinkPerPageBudget {
    current_task_seq_num: usize,
    links_within_current_task: usize,
    allocated_budget: usize,
}

pub struct PageLevel {
    max_level: usize,
}

pub struct MaxRedirect {
    max_redirect: usize
}

#[derive(Default)]
pub struct HashSetDedup {
    visited: std::collections::HashSet<String>,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for SameDomain {
    name!{}
    fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> ExtResult {
        let domain = task.link.url.host_str().unwrap_or("_");

        let root_url =  &ctx.root_url;
        let root_domain = root_url.host_str().unwrap_or("");
        if root_domain.strip_prefix(&self.strip_prefix).unwrap_or(root_domain) == domain.strip_prefix(&self.strip_prefix).unwrap_or(domain)
        {
            return Ok(Action::Accept);
        }
        Ok(Action::Skip)
    }
}

impl SameDomain {
    struct_name!{}
    pub fn new(www_allow: bool) -> Self {
        let mut strip_prefix = String::from("");
        if www_allow {
            strip_prefix = String::from("www.");
        }
        Self { strip_prefix }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for TotalPageBudget {
    name!{}
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, _: &mut rt::Task) -> ExtResult {
        if self.budget >= self.allocated_budget {
            return Err(ExtError::Term)
        }
        self.budget += 1;
        Ok(Action::Accept)
    }
}

impl TotalPageBudget {
    struct_name!{}
    pub fn new(allocated_budget: usize) -> Self {
        Self {
            allocated_budget,
            budget: 0,
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for LinkPerPageBudget {
    name!{}
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, seq_num: usize, _: &mut rt::Task) -> ExtResult {
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
    struct_name!{}
    pub fn new(allocated_budget: usize) -> Self {
        Self {
            allocated_budget,
            current_task_seq_num: 0,
            links_within_current_task: 0,
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for PageLevel {
    name!{}
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> ExtResult {
        if task.level >= self.max_level {
            return Err(ExtError::Term)
        }
        Ok(Action::Accept)
    }
}

impl PageLevel {
    struct_name!{}
    pub fn new(max_level: usize) -> Self {
        Self { max_level }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for HashSetDedup {
    name!{}
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> ExtResult {
        if self.visited.contains(task.link.url.as_str()) {
            return Ok(Action::Skip)
        }

        self.visited.insert(task.link.url.to_string());
        Ok(Action::Accept)
    }
}

impl HashSetDedup {
    struct_name!{}
    pub fn new() -> Self {
        Self {
            visited: std::collections::HashSet::new(),
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for MaxRedirect {
    name!{}
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> ExtResult {
        if task.link.redirect > self.max_redirect {
            return Ok(Action::Skip)
        }
        Ok(Action::Accept)
    }
}

impl MaxRedirect {
    struct_name!{}
    pub fn new(max_redirect: usize) -> Self {
        Self {
            max_redirect
        }
    }
}