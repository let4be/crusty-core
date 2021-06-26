use crate::{_prelude::*, task_expanders::*, types as rt};

pub fn document_parser() -> rt::DocumentParser<Document> {
	Box::new(|reader: Box<dyn io::Read + Sync + Send>| -> rt::Result<Document> {
		Ok(Document { document: select::document::Document::from_read(reader).context("cannot read html document")? })
	})
}

pub struct Document {
	pub(crate) document: select::document::Document,
}

impl Deref for Document {
	type Target = select::document::Document;

	fn deref(&self) -> &Self::Target {
		&self.document
	}
}

impl rt::ParsedDocument for Document {}

pub struct FollowLinks {
	link_target: rt::LinkTarget,
}

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Expander<JS, TS, Document> for FollowLinks {
	name! {}

	fn expand(
		&self,
		ctx: &mut rt::JobCtx<JS, TS>,
		task: &rt::Task,
		_status: &rt::HttpStatus,
		document: &Document,
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

impl<JS: rt::JobStateValues, TS: rt::TaskStateValues> Expander<JS, TS, Document> for LoadImages {
	name! {}

	fn expand(
		&self,
		ctx: &mut rt::JobCtx<JS, TS>,
		task: &rt::Task,
		_status: &rt::HttpStatus,
		document: &Document,
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
