# Crusty-core - build your own web crawler!
 - multi-threaded && async on top of [tokio](https://github.com/tokio-rs/tokio)
 - highly customizable filtering at each and every step - status code/headers received, page downloaded, link filter
 - built on top of [hyper](https://github.com/hyperium/hyper) (http2 and gzip/deflate baked in)  
 - rich content extraction with [select](https://github.com/utkarshkukreti/select.rs)
 - observable with [tracing](https://github.com/tokio-rs/tracing)
 - lots of options, almost everything is configurable
 - applicable both for focused and broad crawling
 - scales with ease when you want to crawl millions/billions of domains
 - it's fast, fast, fast!

### Install

right now API is still in development, once it's a bit settled down I will release on crates.io with `0.1.0`

for now simply add this to your `Cargo.toml`
```
[dependencies]
crusty-core = {git = "https://github.com/let4be/crusty-core"}
```

### Example - crawl single website, collect information about `TITLE` tags 

```rust
use crusty_core::{
    ParserProcessor, CrawlingRules, CrawlingRulesOptions, Crawler, task_expanders::TaskExpander,
    types::{
        Job, JobContext as JobCtx, Task, Status as HttpStatus, JobStatus,
        select::predicate::Name, select::document::Document
    },
    config,
};
use anyhow::{Context as _};

#[derive(Debug, Clone, Default)]
pub struct JobState {
    sum_title_len: usize
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String
}

pub struct DataExtractor {}
impl TaskExpander<JobState, TaskState> for DataExtractor {
    fn expand(&self, ctx: &mut JobCtx<JobState, TaskState>, _: &Task, _: &HttpStatus, doc: &Document) {
        let title = doc.find(Name("title")).next().map(|v|v.text());
        if let Some(title) = title {
            ctx.task_state.lock().unwrap().title = title.clone();
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let concurrency_profile = config::ConcurrencyProfile::default();
    let pp = ParserProcessor::spawn(concurrency_profile, 1024 * 1024 * 32);

    let settings = config::CrawlerSettings::default();
    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let rules_options = CrawlingRulesOptions::default();
    let rules = Box::new(CrawlingRules::new(rules_options)
        .with_task_expanders(|| vec![Box::new(DataExtractor{})] ));

    let url = url::Url::parse("https://bash.im").context("cannot parse url")?;
    let job = Job{url, settings, rules, job_state: JobState::default()};
    let crawler = Crawler::new(networking_profile);
    for r in crawler.iter(job, &pp) {
        println!("- {}, task state: {:?}", r, r.context.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.context.job_state.lock().unwrap());
        }
    }

    Ok(pp.join().await??)
}
```

### Notes

Please see [examples](examples) for more complicated usage scenarios. 
This crawler is more verbose than some others, but it allows incredible customization at each and every step.

If you are interested in the area of broad web crawling there's [crusty](https://github.com/let4be/crusty), developed fully on top of `crusty-core` that tries to tackle on some challenges of broad web crawling