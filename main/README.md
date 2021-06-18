![crates.io](https://img.shields.io/crates/v/crusty-core.svg)
[![Dependency status](https://deps.rs/repo/github/let4be/crusty-core/status.svg)](https://deps.rs/repo/github/let4be/crusty-core)

# Crusty-core - build your own web crawler!

### Example - crawl single website, collect information about `TITLE` tags

```rust
use crusty_core::{prelude::*, select_task_expanders::FollowLinks};

#[derive(Debug, Default)]
pub struct JobState {
    sum_title_len: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String,
}

pub struct DataExtractor {}
type Ctx = JobCtx<JobState, TaskState>;
impl TaskExpander<JobState, TaskState, Document> for DataExtractor {
    fn expand(
        &self,
        ctx: &mut Ctx,
        _: &Task,
        _: &HttpStatus,
        doc: &Document,
    ) -> task_expanders::Result {
        if let Some(title) = doc.find(Name("title")).next().map(|v| v.text()) {
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
            ctx.task_state.title = title;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let crawler = Crawler::new_default()?;

    let settings = config::CrawlingSettings::default();
    let rules = CrawlingRules::new(CrawlingRulesOptions::default(), document_parser())
        .with_task_expander(|| DataExtractor {})
        .with_task_expander(|| FollowLinks::new(LinkTarget::HeadFollow));

    let job = Job::new("https://example.com", settings, rules, JobState::default())?;
    for r in crawler.iter(job) {
        println!("- {}, task state: {:?}", r, r.ctx.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.ctx.job_state.lock().unwrap());
        }
    }
    Ok(())
}

```

If you want to get more fancy and configure some stuff or control your imports more precisely

```rust
use crusty_core::{
    config,
    select::predicate::Name,
    select_task_expanders::{document_parser, Document, FollowLinks},
    task_expanders,
    types::{HttpStatus, Job, JobCtx, JobStatus, LinkTarget, Task},
    Crawler, CrawlingRules, CrawlingRulesOptions, ParserProcessor, TaskExpander,
};

#[derive(Debug, Default)]
pub struct JobState {
    sum_title_len: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String,
}

pub struct DataExtractor {}
type Ctx = JobCtx<JobState, TaskState>;
impl TaskExpander<JobState, TaskState, Document> for DataExtractor {
    fn expand(
        &self,
        ctx: &mut Ctx,
        _: &Task,
        _: &HttpStatus,
        doc: &Document,
    ) -> task_expanders::Result {
        let title = doc.find(Name("title")).next().map(|v| v.text());
        if let Some(title) = title {
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
            ctx.task_state.title = title;
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let concurrency_profile = config::ConcurrencyProfile::default();
    let tx_pp = ParserProcessor::spawn(concurrency_profile, 1024 * 1024 * 32);

    let networking_profile = config::NetworkingProfile::default().resolve()?;
    let crawler = Crawler::new(networking_profile, tx_pp);

    let settings = config::CrawlingSettings::default();
    let rules_opt = CrawlingRulesOptions::default();
    let rules = CrawlingRules::new(rules_opt, document_parser())
        .with_task_expander(|| DataExtractor {})
        .with_task_expander(|| FollowLinks::new(LinkTarget::HeadFollow));

    let job = Job::new("https://example.com", settings, rules, JobState::default())?;
    for r in crawler.iter(job) {
        println!("- {}, task state: {:?}", r, r.ctx.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.ctx.job_state.lock().unwrap());
        }
    }

    Ok(())
}

```

### Install

Simply add this to your `Cargo.toml`
```
[dependencies]
crusty-core = "~0.45.0"
```

if you need just library without built-in `select.rs` task expanders(for links, images, etc)
```
[dependencies]
crusty-core = "~0.45.0"
```

### Key capabilities

- multi-threaded && async on top of [tokio](https://github.com/tokio-rs/tokio)
- highly customizable filtering at each and every step
    - custom dns resolver with builtin IP/subnet filtering
    - status code/headers received(built-in content-type filters work at this step),
    - page downloaded(say we can decide not to parse DOM),
    - task filtering, complete control on -what- to follow and -how- to(just resolve dns, head, head+get)
- built on top of [hyper](https://github.com/hyperium/hyper) (http2 and gzip/deflate baked in)
- rich content extraction with [select](https://github.com/utkarshkukreti/select.rs)
- observable with [tracing](https://github.com/tokio-rs/tracing) and custom metrics exposed to user(stuff like html parsing duration, bytes sent/received)
- lots of options, almost everything is configurable either through options or code
- applicable both for focused and broad crawling
- scales with ease when you want to crawl millions/billions of domains
- it's fast, fast, fast!

### Development

- make sure `rustup` is installed: https://rustup.rs/

- make sure `pre-commit` is installed: https://pre-commit.com/

- make sure `markdown-pp` is installed: https://github.com/jreese/markdown-pp

- run `./go setup`

- run `./go check` to run all pre-commit hooks and ensure everything is ready to go for git

- run `./go release minor` to release a next minor version for crates.io

### Notes

Please see [examples](examples) for more complicated usage scenarios.
This crawler is more verbose than some others, but it allows incredible customization at each and every step.

If you are interested in the area of broad web crawling there's [crusty](https://github.com/let4be/crusty), developed fully on top of `crusty-core` that tries to tackle on some challenges of broad web crawling
