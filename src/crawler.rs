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
    expanders,
    resolver::AsyncHyperResolverAdaptor,
    resolver::Resolver
};

use std::{
    sync::{Arc},
    net::{IpAddr},
};

use futures::{future};
use url::Url;
use tokio::task::JoinHandle;

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
pub struct CrawlingRules<JS, TS> {
    pub options: CrawlingRulesOptions,
    pub custom_task_filters: Option<Box<dyn Fn() -> TaskFilters<JS, TS> + Send + Sync>>,
    pub custom_status_filters: Option<Box<dyn Fn() -> StatusFilters<JS, TS> + Send + Sync>>,
    pub custom_load_filters: Option<Box<dyn Fn() -> LoadFilters<JS, TS> + Send + Sync>>,
    pub custom_task_expanders: Option<Box<dyn Fn() -> TaskExpanders<JS, TS> + Send + Sync>>
}

impl<JS: JobStateValues, TS: TaskStateValues> CrawlingRules<JS, TS> {
    pub fn with_task_filters<T: Fn() -> TaskFilters<JS, TS> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_task_filters = Some(Box::new(f));
        self
    }
    pub fn with_status_filters<T: Fn() -> StatusFilters<JS, TS> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_status_filters = Some(Box::new(f));
        self
    }
    pub fn with_load_filters<T: Fn() -> LoadFilters<JS, TS> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_load_filters = Some(Box::new(f));
        self
    }
    pub fn with_task_expanders<T: Fn() -> TaskExpanders<JS, TS> + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_task_expanders = Some(Box::new(f));
        self
    }
}

impl<JS: JobStateValues, TS: TaskStateValues> JobRules<JS, TS> for CrawlingRules<JS, TS> {
    fn task_filters(&self) -> TaskFilters<JS, TS> {
        let options = &self.options;
        let mut task_filters: TaskFilters<JS, TS> = vec![
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

    fn status_filters(&self) -> StatusFilters<JS, TS> {
        let options = &self.options;

        let mut status_filters: StatusFilters<JS, TS> = vec![];
        if options.accepted_content_types.is_some() {
            status_filters.push(Box::new(status_filters::ContentTypeFilter::new(options.accepted_content_types.clone().unwrap(), true)));
        }
        if options.redirect_term_on_err.is_some() {
            status_filters.push(Box::new(status_filters::RedirectStatusFilter::new(true)));
        }
        status_filters
    }

    fn load_filters(&self) -> LoadFilters<JS, TS> {
        vec![]
    }

    fn task_expanders(&self) -> TaskExpanders<JS, TS> {
        let mut task_expanders : TaskExpanders<JS, TS> = vec![
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
    networking_profile: config::ResolvedNetworkingProfile<R>,
}

type HttpClient<R> = hyper::Client<HttpConnector<R>>;
type HttpConnector<R> = hyper_tls::HttpsConnector<hyper_utils::CountingConnector<hyper::client::HttpConnector<AsyncHyperResolverAdaptor<R>>>>;

impl<R: Resolver> HttpClientFactory<R> {
    fn make(&self) -> (HttpClient<R>, hyper_utils::Stats) {
        let v = self.networking_profile.values.clone();

        let resolver_adaptor = AsyncHyperResolverAdaptor::new(Arc::clone(&self.networking_profile.resolver));
        let mut http = hyper::client::HttpConnector::new_with_resolver(resolver_adaptor);
        http.set_connect_timeout(v.connect_timeout.clone().map(|v| *v));

        match (v.bind_local_ipv4.map(|v|*v), v.bind_local_ipv6.map(|v|*v)) {
            (Some(ipv4), Some(ipv6)) => http.set_local_addresses(ipv4, ipv6),
            (Some(ipv4), None) => http.set_local_address(Some(IpAddr::V4(ipv4))),
            (None, Some(ipv6)) => http.set_local_address(Some(IpAddr::V6(ipv6))),
            (None, None) => {}
        }

        http.set_recv_buffer_size(v.socket_read_buffer_size.clone().map(|v|*v));
        http.set_send_buffer_size(v.socket_write_buffer_size.clone().map(|v|*v));
        http.enforce_http(false);
        let http_counting = hyper_utils::CountingConnector::new(http);
        let stats = http_counting.stats.clone();
        let https = hyper_tls::HttpsConnector::new_with_connector(http_counting);
        let client = hyper::Client::builder().build::<_, hyper::Body>(https);

        (client, stats)
    }
}

pub struct Crawler<JS: JobStateValues, TS: TaskStateValues, R: Resolver>{
    settings: config::CrawlerSettings,
    networking_profile: config::ResolvedNetworkingProfile<R>,
    rules: Arc<BoxedJobRules<JS, TS>>,
}

pub struct CrawlerIter<JS: JobStateValues, TS: TaskStateValues>{
    rx: Receiver<JobUpdate<JS, TS>>,
    _h: JoinHandle<Result<()>>
}

impl<JS: JobStateValues, TS: TaskStateValues> Iterator for CrawlerIter<JS, TS> {
    type Item = JobUpdate<JS, TS>;

    fn next(&mut self) -> Option<Self::Item> {
        futures_lite::future::block_on(self.rx.recv()).ok()
    }
}

impl<JS: JobStateValues, TS: TaskStateValues, R: Resolver> Crawler<JS, TS, R> {
    pub fn new(settings: config::CrawlerSettings, networking_profile: config::ResolvedNetworkingProfile<R>, rules: BoxedJobRules<JS, TS>) -> Crawler<JS, TS, R> {
        Crawler {
            settings,
            networking_profile,
            rules: Arc::new(rules),
        }
    }

    pub fn iter(self, url: Url, job_state: JS, parse_tx: Sender<ParserTask>) -> CrawlerIter<JS, TS> {
        let (tx, rx) = async_channel::unbounded();

        let h = tokio::spawn(async move {
            self.go(url, job_state, parse_tx,tx).await?;
            Ok::<_, Error>(())
        });

        CrawlerIter{rx, _h: h}
    }

    pub fn go(
        &self,
        url: Url,
        job_state: JS,
        parse_tx: Sender<ParserTask>,
        update_tx: Sender<JobUpdate<JS, TS>>,
    ) -> PinnedFut
    {
        TracingTask::new(span!(Level::INFO, url=url.as_str()), async move {
            let ctx = StdJobContext::new(url.clone(), job_state, TS::default());

            let mut scheduler = TaskScheduler::new(
                &url,
                Arc::clone(&self.rules),
                &self.settings,
                ctx.clone(),
                update_tx.clone()
            )?;

            let client_factory = HttpClientFactory {
                networking_profile: self.networking_profile.clone(),
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
        }).instrument()
    }
}

#[derive(Clone)]
pub struct MultiCrawler<JS: JobStateValues, TS: TaskStateValues, R: Resolver> {
    job_rx: Receiver<Job<JS, TS>>,
    update_tx: Sender<JobUpdate<JS, TS>>,
    pp_tx: Sender<ParserTask>,
    concurrency_profile: config::ConcurrencyProfile,
    networking_profile: config::ResolvedNetworkingProfile<R>,
}

pub struct Job<JS: JobStateValues, TS: TaskStateValues> {
    pub url: url::Url,
    pub settings: config::CrawlerSettings,
    pub rules: Box<dyn JobRules<JS, TS>>,
    pub job_state: JS
}

impl<JS: JobStateValues, TS: TaskStateValues, R: Resolver> MultiCrawler<JS, TS, R> {
    pub fn new(pp_tx: Sender<ParserTask>, concurrency_profile: config::ConcurrencyProfile, networking_profile: config::ResolvedNetworkingProfile<R>)
        -> (MultiCrawler<JS, TS, R>, Sender<Job<JS, TS>>, Receiver<JobUpdate<JS, TS>>) {

        let (job_tx, job_rx) = bounded_ch::<Job<JS, TS>>(concurrency_profile.job_tx_buffer_size());
        let (update_tx, update_rx) = bounded_ch::<JobUpdate<JS, TS>>(concurrency_profile.job_update_buffer_size());
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
            let _ = crawler.go(job.url.clone(), job.job_state, self.pp_tx.clone(), self.update_tx.clone()).await;
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