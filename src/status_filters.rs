#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

pub enum StatusFilterAction {
    Skip,
    Term,
}

pub trait StatusFilter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &self,
        ctx: &mut rt::JobContext<JS, TS>,
        task: &rt::Task,
        status: &rt::Status,
    ) -> StatusFilterAction;
}

pub struct ContentTypeFilter {
    accepted: Vec<String>,
    term_on_error: bool
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> StatusFilter<JS, TS> for ContentTypeFilter {
    fn name(&self) -> &'static str { "ContentTypeFilter" }
    fn accept(
        &self,
        _ctx: &mut rt::JobContext<JS, TS>,
        _task: &rt::Task,
        status: &rt::Status,
    ) -> StatusFilterAction {
        let content_type = status.headers.get(http::header::CONTENT_TYPE);
        if content_type.is_none() {
            if self.term_on_error {
                return StatusFilterAction::Term;
            }
            return StatusFilterAction::Skip;
        }
        let content_type = content_type.unwrap().to_str();
        if content_type.is_err() {
            if self.term_on_error {
                return StatusFilterAction::Term;
            }
            return StatusFilterAction::Skip;
        }
        let content_type = content_type.unwrap();

        for ct in &self.accepted {
            if content_type.contains(ct) {
                return StatusFilterAction::Skip;
            }
        }

        return StatusFilterAction::Term;
    }
}

impl ContentTypeFilter {
    pub fn new(accepted: Vec<String>, term_on_error: bool) -> Self {
        Self {accepted, term_on_error }
    }
}

pub struct RedirectStatusFilter {
    term_on_error: bool
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> StatusFilter<JS, TS> for RedirectStatusFilter {
    fn name(&self) -> &'static str { "RedirectLoadFilter" }
    fn accept(
        &self,
        ctx: &mut rt::JobContext<JS, TS>,
        task: &rt::Task,
        status: &rt::Status,
    ) -> StatusFilterAction {
        let sc = status.status_code;
        if sc != 301 && sc != 302 && sc != 303 && sc != 307 {
            return StatusFilterAction::Skip;
        }

        let location = status.headers.get(http::header::LOCATION);
        if location.is_none() {
            if self.term_on_error {
                return StatusFilterAction::Term;
            }
            return StatusFilterAction::Skip;
        }
        let location = location.unwrap().to_str();
        if location.is_err() {
            if self.term_on_error {
                return StatusFilterAction::Term;
            }
            return StatusFilterAction::Skip;
        }
        let location = location.unwrap();

        let link = rt::Link::new(
            String::from(location),
            String::from(""),
            String::from(""),
            task.link.redirect + 1,
            task.link.target.clone(),
            &task.link,
        );

        if link.is_err() {
            if self.term_on_error {
                return StatusFilterAction::Term;
            }
            return StatusFilterAction::Skip;
        }

        ctx.push_links(vec![link.unwrap()]);
        return StatusFilterAction::Skip;
    }
}

impl RedirectStatusFilter {
    pub fn new(term_on_error: bool) -> Self {
        Self {term_on_error}
    }
}