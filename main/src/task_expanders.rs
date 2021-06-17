#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{types as rt, types::SelectDocument};

pub type Result = rt::ExtResult<()>;

pub trait Expander<JS: rt::JobStateValues, TS: rt::TaskStateValues, P> {
	fn name(&self) -> &'static str {
		"no name"
	}
	fn expand(&self, ctx: &mut rt::JobCtx<JS, TS>, task: &rt::Task, status: &rt::HttpStatus, document: &P) -> Result;
}

pub struct FollowLinks {
	link_target: rt::LinkTarget,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Expander<JS, TS, SelectDocument> for FollowLinks {
	name! {}

	fn expand(
		&self,
		ctx: &mut rt::JobCtx<JS, TS>,
		task: &rt::Task,
		_status: &rt::HttpStatus,
		document: &SelectDocument,
	) -> Result {
		let links: Vec<rt::Link> = document
			.find(select::predicate::Name("a"))
			.filter_map(|n| {
				rt::Link::new(
					n.attr("href").unwrap_or(""),
					n.attr("rel").unwrap_or(""),
					n.attr("alt").unwrap_or(""),
					&n.text(),
					0,
					self.link_target,
					&task.link,
				)
				.ok()
			})
			.collect();
		ctx.push_links(links);
		Ok(())
	}
}

impl FollowLinks {
	struct_name! {}

	pub fn new(link_target: rt::LinkTarget) -> Self {
		Self { link_target }
	}
}

pub struct LoadImages {
	link_target: rt::LinkTarget,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Expander<JS, TS, SelectDocument> for LoadImages {
	name! {}

	fn expand(
		&self,
		ctx: &mut rt::JobCtx<JS, TS>,
		task: &rt::Task,
		_status: &rt::HttpStatus,
		document: &SelectDocument,
	) -> Result {
		let links: Vec<rt::Link> = document
			.find(select::predicate::Name("img"))
			.filter_map(|n| {
				rt::Link::new(
					n.attr("src").unwrap_or(""),
					n.attr("rel").unwrap_or(""),
					n.attr("alt").unwrap_or(""),
					&n.text(),
					0,
					self.link_target,
					&task.link,
				)
				.ok()
			})
			.collect();
		ctx.push_links(links);
		Ok(())
	}
}

impl LoadImages {
	struct_name! {}

	pub fn new(link_target: rt::LinkTarget) -> Self {
		Self { link_target }
	}
}
