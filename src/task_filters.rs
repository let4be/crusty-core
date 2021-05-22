#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

#[derive(PartialEq)]
pub enum Result {
    Accept,
    Skip,
    Term,
}

pub trait TaskFilter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &mut self,
        ctx: &mut rt::JobContext<JS, TS>,
        task_seq_num: usize,
        task: &mut rt::Task,
    ) -> Result;
}

pub struct SameDomain {
    strip_prefix: String,
}

pub struct PageBudget {
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
pub struct HashSetDedupTaskFilter {
    visited: std::collections::HashSet<String>,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for SameDomain {
    fn name(&self) -> &'static str {
        "SameDomain"
    }
    fn accept(&mut self, ctx: &mut rt::JobContext<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
        if task.link.target == rt::LinkTarget::Load {
            return Result::Accept;
        }

        let domain = task.link.url.domain().unwrap_or("_");

        let root_url =  &ctx.root_url;
        let root_domain = root_url.domain().unwrap_or("");
        if root_domain.strip_prefix(&self.strip_prefix).unwrap_or(root_domain) == domain.strip_prefix(&self.strip_prefix).unwrap_or(domain)
        {
            return Result::Accept;
        }
        Result::Skip
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

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for PageBudget {
    fn name(&self) -> &'static str {
        "PageBudget"
    }
    fn accept(&mut self, _ctx: &mut rt::JobContext<JS, TS>, _: usize, _: &mut rt::Task) -> Result {
        if self.budget >= self.allocated_budget {
            return Result::Term;
        }
        self.budget += 1;
        Result::Accept
    }
}

impl PageBudget {
    pub fn new(allocated_budget: usize) -> Self {
        Self {
            allocated_budget,
            budget: 0,
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for LinkPerPageBudget {
    fn name(&self) -> &'static str {
        "LinkPerPageBudget"
    }
    fn accept(&mut self, _ctx: &mut rt::JobContext<JS, TS>, seq_num: usize, _: &mut rt::Task) -> Result {
        if seq_num > self.current_task_seq_num {
            self.links_within_current_task = 0;
        }
        self.links_within_current_task += 1;
        if self.links_within_current_task > self.allocated_budget {
            return Result::Term;
        }
        Result::Accept
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

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for PageLevel {
    fn name(&self) -> &'static str {
        "PageLevel"
    }
    fn accept(&mut self, _ctx: &mut rt::JobContext<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
        if task.level >= self.max_level {
            return Result::Term;
        }
        Result::Accept
    }
}

impl PageLevel {
    pub fn new(max_level: usize) -> Self {
        Self { max_level }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for HashSetDedupTaskFilter {
    fn name(&self) -> &'static str {
        "HashSetDedup"
    }
    fn accept(&mut self, _ctx: &mut rt::JobContext<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
        if self.visited.contains(task.link.url.as_str()) {
            return Result::Skip;
        }

        self.visited.insert(task.link.url.to_string());
        Result::Accept
    }
}

impl HashSetDedupTaskFilter {
    pub fn new() -> Self {
        Self {
            visited: std::collections::HashSet::new(),
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for MaxRedirect {
    fn name(&self) -> &'static str {
        "MaxRedirect"
    }
    fn accept(&mut self, _ctx: &mut rt::JobContext<JS, TS>, _: usize, task: &mut rt::Task) -> Result {
        if task.link.redirect > self.max_redirect {
            return Result::Term;
        }
        Result::Accept
    }
}

impl MaxRedirect {
    pub fn new(max_redirect: usize) -> Self {
        Self {
            max_redirect
        }
    }
}