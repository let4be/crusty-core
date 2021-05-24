#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::types as rt;

#[derive(PartialEq)]
pub enum Action {
    Accept,
    Skip,
    Term,
}

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &mut self,
        ctx: &mut rt::JobCtx<JS, TS>,
        task_seq_num: usize,
        task: &mut rt::Task,
    ) -> Action;
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
    fn name(&self) -> &'static str {
        "SameDomain"
    }
    fn accept(&mut self, ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Action {
        if task.link.target == rt::LinkTarget::Load {
            return Action::Accept;
        }

        let domain = task.link.url.domain().unwrap_or("_");

        let root_url =  &ctx.root_url;
        let root_domain = root_url.domain().unwrap_or("");
        if root_domain.strip_prefix(&self.strip_prefix).unwrap_or(root_domain) == domain.strip_prefix(&self.strip_prefix).unwrap_or(domain)
        {
            return Action::Accept;
        }
        Action::Skip
    }
}

impl SameDomain {
    pub fn new(www_allow: bool) -> Self {
        let mut strip_prefix = String::from("");
        if www_allow {
            strip_prefix = String::from("www.");
        }
        Self { strip_prefix }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for TotalPageBudget {
    fn name(&self) -> &'static str {
        "PageBudget"
    }
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, _: &mut rt::Task) -> Action {
        if self.budget >= self.allocated_budget {
            return Action::Term;
        }
        self.budget += 1;
        Action::Accept
    }
}

impl TotalPageBudget {
    pub fn new(allocated_budget: usize) -> Self {
        Self {
            allocated_budget,
            budget: 0,
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for LinkPerPageBudget {
    fn name(&self) -> &'static str {
        "LinkPerPageBudget"
    }
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, seq_num: usize, _: &mut rt::Task) -> Action {
        if seq_num > self.current_task_seq_num {
            self.links_within_current_task = 0;
        }
        self.links_within_current_task += 1;
        if self.links_within_current_task > self.allocated_budget {
            return Action::Term;
        }
        Action::Accept
    }
}

impl LinkPerPageBudget {
    pub fn new(allocated_budget: usize) -> Self {
        Self {
            allocated_budget,
            current_task_seq_num: 0,
            links_within_current_task: 0,
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for PageLevel {
    fn name(&self) -> &'static str {
        "PageLevel"
    }
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Action {
        if task.level >= self.max_level {
            return Action::Term;
        }
        Action::Accept
    }
}

impl PageLevel {
    pub fn new(max_level: usize) -> Self {
        Self { max_level }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for HashSetDedup {
    fn name(&self) -> &'static str {
        "HashSetDedup"
    }
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Action {
        if self.visited.contains(task.link.url.as_str()) {
            return Action::Skip;
        }

        self.visited.insert(task.link.url.to_string());
        Action::Accept
    }
}

impl HashSetDedup {
    pub fn new() -> Self {
        Self {
            visited: std::collections::HashSet::new(),
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for MaxRedirect {
    fn name(&self) -> &'static str {
        "MaxRedirect"
    }
    fn accept(&mut self, _ctx: &mut rt::JobCtx<JS, TS>, _: usize, task: &mut rt::Task) -> Action {
        if task.link.redirect > self.max_redirect {
            return Action::Skip;
        }
        Action::Accept
    }
}

impl MaxRedirect {
    pub fn new(max_redirect: usize) -> Self {
        Self {
            max_redirect
        }
    }
}