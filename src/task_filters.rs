#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

#[derive(PartialEq)]
pub enum TaskFilterResult {
    Accept,
    Skip,
    Term,
}

pub trait TaskFilter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &mut self,
        ctx: &mut rt::StdJobContext<JS, TS>,
        task_seq_num: usize,
        task: &rt::Task,
    ) -> TaskFilterResult;
}

pub struct SameDomainTaskFilter {
    strip_prefix: String,
}

pub struct PageBudgetTaskFilter {
    allocated_budget: usize,
    budget: usize,
}

pub struct LinkPerPageBudgetTaskFilter {
    current_task_seq_num: usize,
    links_within_current_task: usize,
    allocated_budget: usize,
}

pub struct PageLevelTaskFilter {
    max_level: usize,
}

pub struct HashSetDedupTaskFilter {
    visited: std::collections::HashSet<String>,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for SameDomainTaskFilter {
    fn name(&self) -> &'static str {
        "SameDomainTaskFilter"
    }
    fn accept(&mut self, ctx: &mut rt::StdJobContext<JS, TS>, _: usize, task: &rt::Task) -> TaskFilterResult {
        if task.link.target == rt::LinkTarget::Load {
            return TaskFilterResult::Accept;
        }

        let domain = task.link.url.domain().unwrap_or("_");

        let root_url =  &ctx.root_url;
        let root_domain = root_url.domain().unwrap_or("");
        if root_domain.strip_prefix(&self.strip_prefix).unwrap_or(root_domain) == domain.strip_prefix(&self.strip_prefix).unwrap_or(domain)
        {
            return TaskFilterResult::Accept;
        }
        TaskFilterResult::Skip
    }
}

impl SameDomainTaskFilter {
    pub fn new(www_allow: bool) -> Self {
        let mut strip_prefix = String::from("");
        if www_allow {
            strip_prefix = String::from("www.");
        }
        Self { strip_prefix }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for PageBudgetTaskFilter {
    fn name(&self) -> &'static str {
        "PageBudgetTaskFilter"
    }
    fn accept(&mut self, _ctx: &mut rt::StdJobContext<JS, TS>, _: usize, _: &rt::Task) -> TaskFilterResult {
        if self.budget >= self.allocated_budget {
            return TaskFilterResult::Term;
        }
        self.budget += 1;
        TaskFilterResult::Accept
    }
}

impl PageBudgetTaskFilter {
    pub fn new(allocated_budget: usize) -> Self {
        Self {
            allocated_budget,
            budget: 0,
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for LinkPerPageBudgetTaskFilter {
    fn name(&self) -> &'static str {
        "LinkPerPageBudgetTaskFilter"
    }
    fn accept(&mut self, _ctx: &mut rt::StdJobContext<JS, TS>, seq_num: usize, _: &rt::Task) -> TaskFilterResult {
        if seq_num > self.current_task_seq_num {
            self.links_within_current_task = 0;
        }
        self.links_within_current_task += 1;
        if self.links_within_current_task > self.allocated_budget {
            return TaskFilterResult::Term;
        }
        TaskFilterResult::Accept
    }
}

impl LinkPerPageBudgetTaskFilter {
    pub fn new(allocated_budget: usize) -> Self {
        Self {
            allocated_budget,
            current_task_seq_num: 0,
            links_within_current_task: 0,
        }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for PageLevelTaskFilter {
    fn name(&self) -> &'static str {
        "PageLevelTaskFilter"
    }
    fn accept(&mut self, _ctx: &mut rt::StdJobContext<JS, TS>, _: usize, task: &rt::Task) -> TaskFilterResult {
        if task.level >= self.max_level {
            return TaskFilterResult::Term;
        }
        TaskFilterResult::Accept
    }
}

impl PageLevelTaskFilter {
    pub fn new(max_level: usize) -> Self {
        Self { max_level }
    }
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskFilter<JS, TS> for HashSetDedupTaskFilter {
    fn name(&self) -> &'static str {
        "HashSetDedupTaskFilter"
    }
    fn accept(&mut self, _ctx: &mut rt::StdJobContext<JS, TS>, _: usize, task: &rt::Task) -> TaskFilterResult {
        if self.visited.contains(task.link.url.as_str()) {
            return TaskFilterResult::Skip;
        }

        self.visited.insert(task.link.url.clone().to_string());
        TaskFilterResult::Accept
    }
}

impl HashSetDedupTaskFilter {
    pub fn new() -> Self {
        Self {
            visited: std::collections::HashSet::new(),
        }
    }
}
