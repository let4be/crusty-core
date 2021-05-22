#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

pub trait TaskExpander<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn expand(
        &self,
        ctx: &mut rt::JobContext<JS, TS>,
        task: &rt::Task,
        status: &rt::Status,
        document: &select::document::Document,
    );
}

pub struct FollowLinks {
    link_target: rt::LinkTarget
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskExpander<JS, TS> for FollowLinks {
    fn expand(
        &self,
        ctx: &mut rt::JobContext<JS, TS>,
        task: &rt::Task,
        _status: &rt::Status,
        document: &select::document::Document,
    ) {
        let links: Vec<rt::Link> = document
            .find(select::predicate::Name("a"))
            .filter_map(|n| rt::Link::new(
                String::from(n.attr("href").unwrap_or("")),
                String::from(n.attr("alt").unwrap_or("")),
                n.text(),
                0,
                self.link_target.clone(),
                &task.link
                ).ok())
            .collect();
        ctx.push_links(links);
    }
}

impl FollowLinks {
    pub fn new(link_target: rt::LinkTarget) -> Self {
        Self {link_target}
    }
}

pub struct LoadImages {
    link_target: rt::LinkTarget
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> TaskExpander<JS, TS> for LoadImages {
    fn expand(
        &self,
        ctx: &mut rt::JobContext<JS, TS>,
        task: &rt::Task,
        _status: &rt::Status,
        document: &select::document::Document,
    ) {
        let links: Vec<rt::Link> = document
            .find(select::predicate::Name("img"))
            .filter_map(|n| rt::Link::new(
                String::from(n.attr("src").unwrap_or("")),
                String::from(n.attr("alt").unwrap_or("")),
                n.text(),
                0,
                self.link_target.clone(),
                &task.link
            ).ok())
            .collect();
        ctx.push_links(links);
    }
}

impl LoadImages {
    pub fn new(link_target: rt::LinkTarget) -> Self {
        Self {link_target}
    }
}

