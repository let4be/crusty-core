pub use flume::{bounded as ch_bounded, unbounded as ch_unbounded, Receiver as ChReceiver, Sender as ChSender};
#[cfg(feature = "select_rs")]
pub use select::predicate::Name;

#[cfg(feature = "select_rs")]
pub use crate::select_task_expanders::{document_parser, Document};
pub use crate::{
	config, load_filters,
	resolver::AsyncTrustDnsResolver,
	status_filters, task_expanders, task_filters,
	types::{DocumentParser, HttpStatus, Job, JobCtx, JobStatus, JobUpdate, Link, LinkTarget, ParsedDocument, Task},
	Crawler, CrawlingRules, CrawlingRulesOptions, MultiCrawler, ParserActor, TaskExpander,
};
