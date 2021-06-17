use std::io;

use anyhow::Context;
use crusty_core::{
    config, task_expanders, types as rt,
    types::{
        DocumentParser, HttpStatus, Job, JobCtx, JobStatus, Link, LinkTarget, ParsedDocument, Task,
    },
    Crawler, CrawlingRules, CrawlingRulesOptions, ParserProcessor, TaskExpander,
};
use html5ever::{
    local_name,
    tendril::*,
    tokenizer::{
        BufferQueue, CharacterTokens, EndTag, NullCharacterToken, ParseError, StartTag, TagToken,
        Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
    },
};
use markup5ever::{ExpandedName, LocalName, Namespace, Prefix, QualName};

#[derive(Debug, Default)]
pub struct JobState {
    sum_title_len: usize,
}

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    title: String,
}

#[derive(Debug, Default)]
pub struct LinkData {
    href: String,
    alt:  String,
    rel:  String,
}

pub struct Document {
    title: Option<String>,
    links: Vec<LinkData>,
}

impl ParsedDocument for Document {}

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
        let title = doc.title.clone();
        if let Some(title) = title {
            ctx.job_state.lock().unwrap().sum_title_len += title.len();
            ctx.task_state.title = title;
        }
        Ok(())
    }
}

pub struct LinkExtractor {}
impl TaskExpander<JobState, TaskState, Document> for LinkExtractor {
    fn expand(
        &self,
        ctx: &mut Ctx,
        task: &Task,
        _: &HttpStatus,
        doc: &Document,
    ) -> task_expanders::Result {
        let mut links = vec![];
        for link in &doc.links {
            if let Some(link) = Link::new(
                &link.href,
                &link.rel,
                &link.alt,
                "",
                0,
                LinkTarget::HeadFollow,
                &task.link,
            )
            .ok()
            {
                println!("found a link {}", &link);
                links.push(link);
            }
        }
        ctx.push_links(links);
        Ok(())
    }
}

#[derive(Default)]
struct TokenCollector {
    links: Vec<LinkData>,
}

impl TokenCollector {}

impl TokenSink for TokenCollector {
    type Handle = ();

    fn process_token(&mut self, token: Token, _line_number: u64) -> TokenSinkResult<()> {
        match token {
            CharacterTokens(b) => {}
            TagToken(tag) => {
                if tag.name == local_name!("a") {
                    let mut link = LinkData::default();
                    for attr in tag.attrs {
                        match attr.name.local {
                            local_name!("href") => link.href = attr.value.to_string(),
                            local_name!("rel") => link.rel = attr.value.to_string(),
                            local_name!("alt") => link.alt = attr.value.to_string(),
                            _ => {}
                        }
                    }
                    self.links.push(link)
                }
            }
            ParseError(err) => {}
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn document_parser() -> DocumentParser<Document> {
    Box::new(|mut reader: Box<dyn io::Read + Sync + Send>| -> rt::Result<Document> {
        let mut sink = TokenCollector::default();
        let mut chunk = ByteTendril::new();
        reader.read_to_tendril(&mut chunk).context("cannot read")?;
        let mut input = BufferQueue::new();
        input.push_back(chunk.try_reinterpret().unwrap());

        let mut tok = Tokenizer::new(sink, TokenizerOpts { profile: true, ..Default::default() });
        let _ = tok.feed(&mut input);
        tok.end();

        Ok(Document { title: None, links: tok.sink.links })
    })
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
        .with_task_expander(|| LinkExtractor {});

    let job = Job::new("https://example.com", settings, rules, JobState::default())?;
    for r in crawler.iter(job) {
        println!("- {}, task state: {:?}", r, r.ctx.task_state);
        if let JobStatus::Finished(_) = r.status {
            println!("final job state: {:?}", r.ctx.job_state.lock().unwrap());
        }
    }

    Ok(())
}
