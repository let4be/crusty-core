#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    types::*,
    task_scheduler::*,
    task_processor::*,
    config,
    task_filters,
    hyper_utils,
    status_filters,
    load_filters,
    expanders,
    resolver::AsyncHyperResolverAdaptor,
    resolver::Resolver
};

use std::{
    sync::{Arc},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    marker::PhantomData
};

use futures::{future};
use url::Url;

pub type TaskFilters<JobState, TaskState> = Vec<Box<dyn task_filters::TaskFilter<JobState, TaskState> + Send + Sync>>;
pub type StatusFilters<JobState, TaskState> = Vec<Box<dyn status_filters::StatusFilter<JobState, TaskState> + Send + Sync>>;
pub type LoadFilters<JobState, TaskState> = Vec<Box<dyn load_filters::LoadFilter<JobState, TaskState> + Send + Sync>>;
pub type TaskExpanders<JobState, TaskState> = Vec<Box<dyn expanders::TaskExpander<JobState, TaskState> + Send + Sync>>;

pub trait JobRules<JobState: JobStateValues, TaskState: TaskStateValues>: Send + Sync + 'static {
    fn task_filters(&self) -> TaskFilters<JobState, TaskState>;
    fn status_filters(&self) -> StatusFilters<JobState, TaskState>;
    fn load_filters(&self) -> LoadFilters<JobState, TaskState>;
    fn task_expanders(&self) -> TaskExpanders<JobState, TaskState>;
}

#[derive(Clone)]
pub struct CrawlingRulesOptions {
    pub allow_www: bool,
    pub page_budget: Option<usize>,
    pub links_per_page_budget: Option<usize>,
    pub max_level: Option<usize>,
    pub accepted_content_types: Option<Vec<String>>,
    pub redirect_term_on_err: Option<bool>
}

impl Default for CrawlingRulesOptions{
    fn default() -> Self {
        Self {
            allow_www: true,
            page_budget: Some(50),
            links_per_page_budget: Some(50),
            max_level: Some(10),
            accepted_content_types: Some(vec![String::from("text/html")]),
            redirect_term_on_err: Some(true),
        }
    }
}

#[derive(Default)]
pub struct CrawlingRules<JobState, TaskState> {
    pub options: CrawlingRulesOptions,
    pub custom_task_filters: Option<Box<dyn Fn() -> TaskFilters<JobState, TaskState> + Send + Sync>>,
    pub custom_status_filters: Option<Box<dyn Fn() -> StatusFilters<JobState, TaskState> + Send + Sync>>,
    pub custom_load_filters: Option<Box<dyn Fn() -> LoadFilters<JobState, TaskState> + Send + Sync>>,
    pub custom_task_expanders: Option<Box<dyn Fn() -> TaskExpanders<JobState, TaskState> + Send + Sync>>
}

impl<JobState: JobStateValues, TaskState: TaskStateValues> CrawlingRules<JobState, TaskState> {
    pub fn with_custom_task_filters<T: Fn() -> TaskFilters<JobState, TaskState> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_task_filters = Some(Box::new(f));
        self
    }
    pub fn with_custom_status_filters<T: Fn() -> StatusFilters<JobState, TaskState> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_status_filters = Some(Box::new(f));
        self
    }
    pub fn with_custom_load_filters<T: Fn() -> LoadFilters<JobState, TaskState> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_load_filters = Some(Box::new(f));
        self
    }
    pub fn with_custom_task_expanders<T: Fn() -> TaskExpanders<JobState, TaskState> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_task_expanders = Some(Box::new(f));
        self
    }
}

impl<JobState: JobStateValues, TaskState: TaskStateValues> JobRules<JobState, TaskState> for CrawlingRules<JobState, TaskState> {
    fn task_filters(&self) -> TaskFilters<JobState, TaskState> {
        let options = &self.options;
        let mut task_filters: TaskFilters<JobState, TaskState> = vec![
            Box::new(task_filters::SameDomainTaskFilter::new(options.allow_www)),
            Box::new(task_filters::HashSetDedupTaskFilter::new()),
        ];
        if options.page_budget.is_some() {
            task_filters.push(Box::new(task_filters::PageBudgetTaskFilter::new(options.page_budget.unwrap())));
        }
        if options.links_per_page_budget.is_some() {
            task_filters.push(Box::new(task_filters::LinkPerPageBudgetTaskFilter::new(options.links_per_page_budget.unwrap())))
        }
        if options.max_level.is_some() {
            task_filters.push(Box::new(task_filters::PageLevelTaskFilter::new(options.max_level.unwrap())))
        }

        task_filters
    }

    fn status_filters(&self) -> StatusFilters<JobState, TaskState> {
        let options = &self.options;

        let mut status_filters: StatusFilters<JobState, TaskState> = vec![];
        if options.accepted_content_types.is_some() {
            status_filters.push(Box::new(status_filters::ContentTypeFilter::new(options.accepted_content_types.clone().unwrap(), true)));
        }
        if options.redirect_term_on_err.is_some() {
            status_filters.push(Box::new(status_filters::RedirectStatusFilter::new(true)));
        }
        status_filters
    }

    fn load_filters(&self) -> LoadFilters<JobState, TaskState> {
        vec![]
    }

    fn task_expanders(&self) -> TaskExpanders<JobState, TaskState> {
        let mut task_expanders : TaskExpanders<JobState, TaskState> = vec![
            Box::new(expanders::FollowLinks::new()),
        ];
        if self.custom_task_expanders.is_some() {
            let custom_task_expanders = self.custom_task_expanders.as_ref().unwrap();
            for e in custom_task_expanders() {
                task_expanders.push(e);
            }
        }

        task_expanders
    }
}

#[derive(Clone)]
struct HttpClientFactory<R: Resolver> {
    settings: config::CrawlerSettings,
    addr_ipv4: Option<Ipv4Addr>,
    addr_ipv6: Option<Ipv6Addr>,
    resolver: Arc<R>,
}

type HttpClient<R> = hyper::Client<HttpConnector<R>>;
type HttpConnector<R> = hyper_tls::HttpsConnector<hyper_utils::CountingConnector<hyper::client::HttpConnector<AsyncHyperResolverAdaptor<R>>>>;

impl<R: Resolver> HttpClientFactory<R> {
    fn make(&self) -> (HttpClient<R>, hyper_utils::Stats) {
        let resolver = AsyncHyperResolverAdaptor::new(self.resolver.clone());
        let mut http = hyper::client::HttpConnector::new_with_resolver(resolver);
        http.set_connect_timeout(self.settings.connect_timeout.clone().map(|v| *v));

        if self.addr_ipv4.is_some() ^ self.addr_ipv6.is_some() {
            if self.addr_ipv4.is_some() {
                http.set_local_address(Some(IpAddr::V4(self.addr_ipv4.unwrap())));
            } else if self.addr_ipv6.is_some() {
                http.set_local_address(Some(IpAddr::V6(self.addr_ipv6.unwrap())));
            }
        } else if self.addr_ipv4.is_some() && self.addr_ipv6.is_some() {
            http.set_local_addresses(self.addr_ipv4.unwrap(), self.addr_ipv6.unwrap());
        }

        http.set_recv_buffer_size(self.settings.socket_read_buffer_size.clone().map(|v|*v));
        http.set_send_buffer_size(self.settings.socket_write_buffer_size.clone().map(|v|*v));
        http.enforce_http(false);
        let http_counting = hyper_utils::CountingConnector::new(http);
        let stats = http_counting.stats.clone();
        let https = hyper_tls::HttpsConnector::new_with_connector(http_counting);
        let client = hyper::Client::builder().build::<_, hyper::Body>(https);

        (client, stats)
    }
}

#[derive(Clone, Debug)]
pub struct Crawler<JobState: JobStateValues, TaskState: TaskStateValues, R: Resolver, RR: JobRules<JobState, TaskState>>{
    settings: config::CrawlerSettings,
    networking_profile: config::NetworkingProfile<R>,
    rules: Arc<RR>,
    _js: PhantomData<JobState>,
    _ts: PhantomData<TaskState>
}

impl<JobState: JobStateValues, TaskState: TaskStateValues, R: Resolver, RR: JobRules<JobState, TaskState>> Crawler<JobState, TaskState, R, RR> {
    pub fn new(settings: config::CrawlerSettings, networking_profile: config::NetworkingProfile<R>, rules: RR) -> Crawler<JobState, TaskState, R, RR> {
        Crawler {
            settings,
            networking_profile,
            rules: Arc::new(rules),
            _js: PhantomData::default(),
            _ts: PhantomData::default(),
        }
    }

    pub fn go(
        &self,
        url: Url,
        parse_tx: Sender<ParserTask>,
        update_tx: Sender<JobUpdate<JobState, TaskState>>,
    ) -> Result<PinnedFut>
    {
        let resolver = if self.networking_profile.resolver.is_none() {
            Some(Arc::new(Resolver::new_default().context("cannot create default resolver")?))
        } else {
            self.networking_profile.resolver.clone()
        }.unwrap();

        Ok(TracingTask::new(span!(Level::INFO, url=url.as_str()), async move {
            let ctx = StdJobContext::new(url.clone(), JobState::default(), TaskState::default());

            let mut scheduler = TaskScheduler::new(
                &url,
                Arc::clone(&self.rules),
                &self.settings,
                ctx.clone(),
                update_tx.clone()
            )?;

            let client_factory = HttpClientFactory {
                settings: self.settings.clone(),
                resolver: Arc::clone(&resolver),
                addr_ipv4: self.networking_profile.bind_local_ipv4.clone().map(|v|*v),
                addr_ipv6: self.networking_profile.bind_local_ipv6.clone().map(|v|*v),
            };

            let mut processor_handles = vec![];
            for i in 0..self.settings.concurrency {
                let client_factory = (move |cf: HttpClientFactory<R>|
                    Box::new(move || cf.make())
                )(client_factory.clone());

                let mut processor = TaskProcessor::new(
                    &url,
                    Arc::clone(&self.rules),
                    &self.settings,
                    ctx.clone(),
                    scheduler.job_update_tx.clone(),
                    scheduler.tasks_rx.clone(),
                    parse_tx.clone(),
                    client_factory
                );

                processor_handles.push(tokio::spawn(async move {
                    processor.go(i).await
                }));
            }

            let job_finishing_update = scheduler.go().await?;
            drop(scheduler);

            trace!("Waiting for workers to finish...");
            futures::future::join_all(processor_handles).await;
            trace!("Workers are done - sending notification about job done!");

            let _ = update_tx.send(job_finishing_update).await;
            Ok(())
        }).instrument())
    }
}

#[derive(Clone)]
pub struct MultiCrawler<JobState: JobStateValues, TaskState: TaskStateValues, R: Resolver, RR: JobRules<JobState, TaskState>> {
    job_rx: Receiver<Job<JobState, TaskState, RR>>,
    update_tx: Sender<JobUpdate<JobState, TaskState>>,
    pp_tx: Sender<ParserTask>,
    concurrency_profile: config::ConcurrencyProfile,
    networking_profile: config::NetworkingProfile<R>,
}

pub struct Job<JobState: JobStateValues, TaskState: TaskStateValues, RR: JobRules<JobState, TaskState> + Send + Sync> {
    pub ctx: StdJobContext<JobState, TaskState>,
    pub url: url::Url,
    pub settings: config::CrawlerSettings,
    pub rules: RR,
}

impl<JobState: JobStateValues, TaskState: TaskStateValues, R: Resolver, RR: JobRules<JobState, TaskState> + Send + Sync + Clone> MultiCrawler<JobState, TaskState, R, RR> {
    pub fn new(pp_tx: Sender<ParserTask>, concurrency_profile: config::ConcurrencyProfile, networking_profile: config::NetworkingProfile<R>)
        -> (MultiCrawler<JobState, TaskState, R, RR>, Sender<Job<JobState, TaskState, RR>>, Receiver<JobUpdate<JobState, TaskState>>) {

        let (job_tx, job_rx) = bounded_ch::<Job<JobState, TaskState, RR>>(concurrency_profile.job_tx_buffer_size());
        let (update_tx, update_rx) = bounded_ch::<JobUpdate<JobState, TaskState>>(concurrency_profile.job_update_buffer_size());
        (MultiCrawler {
            job_rx,
            update_tx,
            pp_tx,
            concurrency_profile,
            networking_profile,
        }, job_tx, update_rx)
    }

    async fn process(&self) -> Result<()> {
        while let Ok(job) = self.job_rx.recv().await {
            let crawler = Crawler::new(job.settings.clone(), self.networking_profile.clone(), job.rules);
            let _ = crawler.go(job.url.clone(), self.pp_tx.clone(), self.update_tx.clone())?.await;
        }
        Ok(())
    }

    pub fn go(self) -> PinnedFut<'static> {
        TracingTask::new(span!(Level::INFO), async move {
            let mut handles = vec![];
            for _ in 0..self.concurrency_profile.domain_concurrency {
                let h = tokio::spawn({
                    let s = self.clone();
                    async move {
                        s.process().await?;
                        Ok::<_, Error>(())
                    }
                });
                handles.push(h);
            }

            let _ = future::join_all(handles).await;
            Ok(())
        }).instrument()
    }
}