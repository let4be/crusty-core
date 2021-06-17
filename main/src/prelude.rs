pub use flume::{bounded as ch_bounded, unbounded as ch_unbounded, Receiver as ChReceiver, Sender as ChSender};
pub use select::{document::Document, predicate::Name};

pub use crate::{
	config, load_filters,
	resolver::AsyncHyperResolver,
	select_document_parser, status_filters, task_expanders, task_filters,
	types::{
		DocumentParser, HttpStatus, Job, JobCtx, JobStatus, JobUpdate, Link, LinkTarget, ParsedDocument,
		SelectDocument, Task,
	},
	Crawler, CrawlingRules, CrawlingRulesOptions, MultiCrawler, ParserProcessor, TaskExpander,
};
