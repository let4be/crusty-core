pub use flume::{bounded as ch_bounded, unbounded as ch_unbounded, Receiver as ChReceiver, Sender as ChSender};
pub use select::{document::Document, predicate::Name};

pub use crate::{
	config, load_filters,
	resolver::AsyncHyperResolver,
	status_filters, task_expanders, task_filters,
	types::{HttpStatus, Job, JobCtx, JobStatus, JobUpdate, Link, LinkTarget, Task},
	Crawler, CrawlingRules, CrawlingRulesOptions, MultiCrawler, ParserProcessor, TaskExpander,
};
