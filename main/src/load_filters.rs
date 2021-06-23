#[allow(unused_imports)]
use crate::_prelude::*;
use crate::types as rt;

pub type Result = rt::ExtResult<()>;
pub static CONTENT_TYPE_TERM_REASON: &str = "ContentType";

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
	fn name(&self) -> &'static str {
		"no name"
	}
	fn accept(
		&self,
		ctx: &rt::JobCtx<JS, TS>,
		task: &rt::Task,
		status: &rt::HttpStatus,
		reader: Box<dyn io::Read + Sync + Send>,
	) -> Result;
}

pub struct ContentType<'a> {
	accepted: Vec<&'a str>,
}

impl<'a, JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for ContentType<'a> {
	name! {}

	fn accept(
		&self,
		_ctx: &rt::JobCtx<JS, TS>,
		_task: &rt::Task,
		status: &rt::HttpStatus,
		_reader: Box<dyn io::Read + Sync + Send>,
	) -> Result {
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

#[derive(Default)]
pub struct RobotsTxt {}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Filter<JS, TS> for RobotsTxt {
	fn accept(
		&self,
		ctx: &rt::JobCtx<JS, TS>,
		task: &rt::Task,
		status: &rt::HttpStatus,
		mut reader: Box<dyn io::Read + Sync + Send>,
	) -> Result {
		if task.link.url.as_str().ends_with("robots.txt") {
			let content_type = status
				.headers
				.get(http::header::CONTENT_TYPE)
				.map(|v| v.to_str())
				.unwrap_or_else(|| Ok(""))
				.unwrap_or("");
			if content_type.to_lowercase() == "text/plain" {
				let mut content = String::from("");
				let _ = reader.read_to_string(&mut content).context("cannot read robots.txt")?;

				ctx.shared.lock().unwrap().insert(String::from("robots"), Box::new(content));
			}
		}

		Ok(())
	}
}

impl RobotsTxt {
	pub fn new() -> Self {
		Self::default()
	}
}
