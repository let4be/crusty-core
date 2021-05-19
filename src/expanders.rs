#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

pub trait TaskExpander<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn expand(
        &self,
        ctx: &mut rt::StdJobContext<JS, TS>,
        task: &rt::Task,
        status: &rt::Status,
        document: &select::document::Document,
    );
}

pub struct FollowLinks {}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskExpander<JS, TS> for FollowLinks {
    fn expand(
        &self,
        ctx: &mut rt::StdJobContext<JS, TS>,
        task: &rt::Task,
        _status: &rt::Status,
        document: &select::document::Document,
    ) {
        let links: Vec<rt::Link> = document
            .find(select::predicate::Name("a"))
            .filter_map(|n| rt::Link::new(
                String::from(n.attr("href").unwrap_or("")),
                String::from(n.attr("alt").unwrap_or("")),
                n.text().trim().to_string(),
                0,
                rt::LinkTarget::Follow,
                &task.link
                ).ok())
            .collect();
        ctx.push_links(links);
    }
}

impl FollowLinks {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct LoadImages {}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskExpander<JS, TS> for LoadImages {
    fn expand(
        &self,
        ctx: &mut rt::StdJobContext<JS, TS>,
        task: &rt::Task,
        _status: &rt::Status,
        document: &select::document::Document,
    ) {
        let links: Vec<rt::Link> = document
            .find(select::predicate::Name("img"))
            .filter_map(|n| rt::Link::new(
                String::from(n.attr("src").unwrap_or("")),
                String::from(n.attr("alt").unwrap_or("")),
                n.text().trim().to_string(),
                0,
                rt::LinkTarget::Load,
                &task.link
            ).ok())
            .collect();
        ctx.push_links(links);
    }
}

impl LoadImages {
    pub fn new() -> Self {
        Self {}
    }
}

