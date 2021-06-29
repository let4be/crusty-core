use crate::{_prelude::*, types as rt, types::ExtError};

pub type Result = rt::ExtResult<()>;

pub static REDIRECT_TERM_REASON: &str = "Redirect";
pub static MAX_REDIRECT_TERM_REASON: &str = "MaxRedirect";
pub static CONTENT_TYPE_TERM_REASON: &str = "ContentType";

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
	fn name(&self) -> &'static str {
		"no name"
	}
	fn accept(&self, ctx: &mut rt::JobCtx<JS, TS>, task: &rt::Task, status: &rt::HttpStatus) -> Result;
}

pub struct ContentType<'a> {
	accepted: Vec<&'a str>,
}

impl<'a, JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for ContentType<'a> {
	name! {}

	fn accept(&self, _ctx: &mut rt::JobCtx<JS, TS>, _task: &rt::Task, status: &rt::HttpStatus) -> Result {
		let content_type = status.headers.get_str(http::header::CONTENT_TYPE)?;
		for accepted in &self.accepted {
			if content_type.contains(accepted) {
				return Ok(())
			}
		}
		Err(rt::ExtError::Term { reason: CONTENT_TYPE_TERM_REASON })
	}
}

impl<'a> ContentType<'a> {
	struct_name! {}

	pub fn new(accepted: Vec<&'a str>) -> Self {
		Self { accepted }
	}
}

pub struct Redirect {
	max_redirects: usize,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for Redirect {
	name! {}

	fn accept(&self, ctx: &mut rt::JobCtx<JS, TS>, task: &rt::Task, status: &rt::HttpStatus) -> Result {
		if !(300..400).contains(&status.code) {
			return Ok(())
		}

		if task.link.redirect >= self.max_redirects {
			return Err(ExtError::Term { reason: MAX_REDIRECT_TERM_REASON })
		}

		let location = status.headers.get_str(http::header::LOCATION)?;

		let mut link = rt::Link::new(location, "", "", "", task.link.redirect + 1, task.link.target, &task.link)
			.context("cannot create link")?;
		link.marker = task.link.marker;

		ctx.push_links(vec![link]);
		Err(ExtError::Term { reason: REDIRECT_TERM_REASON })
	}
}

impl Redirect {
	struct_name! {}

	pub fn new(max_redirects: usize) -> Self {
		Self { max_redirects }
	}
}
