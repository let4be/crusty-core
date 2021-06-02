#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::types as rt;

pub type ExtResult = rt::ExtResult<()>;

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
	fn name(&self) -> String {
		String::from("no name")
	}
	fn accept(&self, ctx: &mut rt::JobCtx<JS, TS>, task: &rt::Task, status: &rt::HttpStatus) -> ExtResult;
}

pub struct ContentType {
	accepted: Vec<String>,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for ContentType {
	name! {}

	fn accept(&self, _ctx: &mut rt::JobCtx<JS, TS>, _task: &rt::Task, status: &rt::HttpStatus) -> ExtResult {
		let content_type = status
			.headers
			.get(http::header::CONTENT_TYPE)
			.ok_or_else(|| anyhow!("content-type: not found"))?
			.to_str()
			.context("cannot read content-type value")?;
		for accepted in &self.accepted {
			if content_type.contains(accepted) {
				return Ok(());
			}
		}
		Err(rt::ExtError::Term)
	}
}

impl ContentType {
	struct_name! {}

	pub fn new(accepted: Vec<String>) -> Self {
		Self { accepted }
	}
}

#[derive(Default)]
pub struct Redirect {}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for Redirect {
	name! {}

	fn accept(&self, ctx: &mut rt::JobCtx<JS, TS>, task: &rt::Task, status: &rt::HttpStatus) -> ExtResult {
		let sc = status.code;
		if sc != 301 && sc != 302 && sc != 303 && sc != 307 {
			return Ok(());
		}

		let location = status
			.headers
			.get(http::header::LOCATION)
			.ok_or_else(|| anyhow!("location: not found"))?
			.to_str()
			.context("cannot read location value")?;

		let link = rt::Link::new(
			String::from(location),
			String::from(""),
			String::from(""),
			task.link.redirect + 1,
			task.link.target,
			&task.link,
		)
		.context("cannot create link")?;

		ctx.push_links(vec![link]);
		Ok(())
	}
}

impl Redirect {
	struct_name! {}

	pub fn new() -> Self {
		Self::default()
	}
}
