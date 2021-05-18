# Crusty-core - build your own web crawler!
 - multi-threaded && async on top of [tokio](https://github.com/tokio-rs/tokio)
 - highly customizable filtering at each and every step - status code/headers received, page downloaded, link filter
 - built on top of [hyper](https://github.com/hyperium/hyper) for additional speed and customizability  
 - rich content extraction with [select](https://github.com/utkarshkukreti/select.rs)
 - observable with [tracing](https://github.com/tokio-rs/tracing)
 - lots of options, almost everything is configurable
 - applicable both for focused and broad crawling
 - scales with ease when you want to crawl millions/billions of domains
 - it's fast, fast, fast!

### Install

right now API is in active development, once it's a bit settled down I will release on crates.io with `0.1.0`

for now simply add this to your `Cargo.toml`
```
[dependencies]
crusty-core = {git = "https://github.com/let4be/crusty-core"}
```

### Example - crawl single website, collect information about `TITLE` tags 

```rust
use crusty_core::{
    ParserProcessor,
    CrawlingRules,
    CrawlingRulesOptions,
    Crawler,
    expanders::TaskExpander,
    types::{
        JobStateValues, StdJobContext, Task, Status as HttpStatus, JobStatus,
        select::predicate::Name, select::document::Document
    },
    config,
};

use anyhow::{Context as _};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct JobState {
    sum_title_len: usize
}
impl JobStateValues for JobState {}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String
}

pub struct DataExtractor {}
impl TaskExpander<JobState, TaskState> for DataExtractor {
    fn expand(
        &self, ctx: &mut StdJobContext<JobState, TaskState>,
        _task: &Task, _status: &HttpStatus, document: &Document
    ) {
        let title = document.find(Name("title")).next().map(|v|v.text());
        if title.is_some() {
            let title = title.unwrap();
            ctx.task_state.lock().unwrap().title = title.clone();
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let concurrency_profile = config::ConcurrencyProfile::default();
    let (pp, tx_pp) = ParserProcessor::new( concurrency_profile, 1024 * 1024 * 32);
    let h_pp = tokio::spawn(pp.go());

    let crawler_settings = config::CrawlerSettings::default();
    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let rules = Box::new(CrawlingRules {
        options: CrawlingRulesOptions::default(),
        ..CrawlingRules::default()}
        .with_task_expanders(|| vec![Box::new(DataExtractor{})] ));

    let crawler = Crawler::<JobState, TaskState, _>::new(crawler_settings, networking_profile, rules);

    let url = Url::parse("https://bash.im").context("cannot parse url")?;
    for r in crawler.iter(url, JobState::default(), tx_pp) {
        println!("- {}, task context: {:?}", r, r.context.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final context: {:?}", r.context.job_state.lock().unwrap());
        }
    }

    Ok(h_pp.await??)
}
```

Please see [examples](examples) for more complicated usage scenarios. 
This crawler is way more verbose than some others, but it allows incredible customization at each and every step.