#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::types as rt;

pub type Result = rt::ExtResult<()>;

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
	fn name(&self) -> String {
		String::from("no name")
	}
	fn accept(
		&self,
		ctx: &rt::JobCtx<JS, TS>,
		task: &rt::Task,
		status: &rt::HttpStatus,
		reader: Box<dyn io::Read + Sync + Send>,
	) -> Result;
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
