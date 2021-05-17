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
};

use futures::{future};
use url::Url;
use crate::resolver::AsyncHyperResolver;

pub trait JobRules<T: JobContextValues> {
    fn task_filters(&self) -> Vec<Box<dyn task_filters::TaskFilter<T> + Send + Sync>>;
    fn status_filters(&self) -> Vec<Box<dyn status_filters::StatusFilter<T> + Send + Sync>>;
    fn load_filters(&self) -> Vec<Box<dyn load_filters::LoadFilter<T> + Send + Sync>>;
    fn task_expanders(&self) -> Vec<Box<dyn expanders::TaskExpander<T> + Send + Sync>>;
}

pub struct DefaultCrawlingRules {
    allow_www: bool,
    page_budget: Option<usize>,
    links_per_page_budget: Option<usize>,
    max_level: Option<usize>,
    accepted_content_types: Option<Vec<String>>,
    redirect_term_on_err: Option<bool>
}

impl Default for DefaultCrawlingRules{
    fn default() -> Self {
        Self {
            allow_www: true,
            page_budget: Some(50),
            links_per_page_budget: Some(50),
            max_level: Some(10),
            accepted_content_types: Some(vec![String::from("text/html")]),
            redirect_term_on_err: Some(true)
        }
    }
}

impl<T: JobContextValues> JobRules<T> for DefaultCrawlingRules {
    fn task_filters(&self) -> Vec<Box<dyn task_filters::TaskFilter<T> + Send + Sync>> {
        let mut filters : Vec<Box<dyn task_filters::TaskFilter<T> + Send + Sync>> = vec![
            Box::new(task_filters::SameDomainTaskFilter::new(self.allow_www)),
            Box::new(task_filters::HashSetDedupTaskFilter::new()),
        ];
        if self.page_budget.is_some() {
            filters.push(Box::new(task_filters::PageBudgetTaskFilter::new(self.page_budget.unwrap())));
        }
        if self.links_per_page_budget.is_some() {
            filters.push(Box::new(task_filters::LinkPerPageBudgetTaskFilter::new(self.links_per_page_budget.unwrap())))
        }
        if self.max_level.is_some() {
            filters.push(Box::new(task_filters::PageLevelTaskFilter::new(self.max_level.unwrap())))
        }

        filters
    }

    fn status_filters(&self) -> Vec<Box<dyn status_filters::StatusFilter<T> + Send + Sync>> {
        let mut filters: Vec<Box<dyn status_filters::StatusFilter<T> + Send + Sync>> = vec![];
        if self.accepted_content_types.is_some() {
            filters.push(Box::new(status_filters::ContentTypeFilter::new(self.accepted_content_types.clone().unwrap(), true)));
        }
        if self.redirect_term_on_err.is_some() {
            filters.push(Box::new(status_filters::RedirectStatusFilter::new(true)));
        }
        filters
    }

    fn load_filters(&self) -> Vec<Box<dyn load_filters::LoadFilter<T> + Send + Sync>> {
        vec![]
    }

    fn task_expanders(&self, ) -> Vec<Box<dyn expanders::TaskExpander<T> + Send + Sync>> {
        vec![
            //Box::new(expanders::LoadImages::new()),
            Box::new(expanders::FollowLinks::new()),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct Crawler<R: Resolver=AsyncHyperResolver> {
    url: Url,
    settings: config::CrawlerSettings,
    networking_profile: config::NetworkingProfile,
    resolver: Option<Arc<R>>
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

impl<R: Resolver> Crawler<R> {
    pub fn new(url: Url, settings: config::CrawlerSettings, networking_profile: config::NetworkingProfile) -> Crawler<R> {
        Crawler {
            url,
            settings,
            networking_profile,
            resolver: None,
        }
    }

    pub fn set_name_resolver(&mut self, r: Arc<R>) {
        self.resolver = Some(r);
    }

    pub fn go<T: JobContextValues>(
        &self,
        rules: Box<dyn JobRules<T> + Send + Sync>,
        parse_tx: Sender<ParserTask>,
        sub_tx: Sender<JobUpdate<T>>,
        ctx: StdJobContext<T>
    ) -> Result<PinnedFut>
    {
        let resolver = if self.resolver.is_none() {
            Some(Arc::new(R::new_default().context("cannot create default resolver")?))
        } else {
            self.resolver.clone()
        }.unwrap();

        Ok(TracingTask::new(span!(Level::INFO, url=self.url.as_str()), async move {
            let mut scheduler = TaskScheduler::new(
                &self.url,
                &self.settings,
                rules.task_filters(),
                ctx.clone(),
                sub_tx.clone()
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
                    &self.url,
                    &self.settings,
                    rules.status_filters(),
                    rules.load_filters(),
                    rules.task_expanders(),
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

            let _ = sub_tx.send(job_finishing_update).await;
            Ok(())
        }).instrument())
    }
}

#[derive(Clone)]
pub struct MultiCrawler<T: JobContextValues, R: Resolver> {
    job_rx: Receiver<Job<T>>,
    update_tx: Sender<JobUpdate<T>>,
    pp_tx: Sender<ParserTask>,
    concurrency_profile: config::ConcurrencyProfile,
    networking_profile: config::NetworkingProfile,
    resolver: Option<Arc<R>>,
}

pub struct Job<T: JobContextValues> {
    pub ctx: StdJobContext<T>,
    pub url: url::Url,
    pub settings: config::CrawlerSettings,
    pub rules: Box<dyn JobRules<T> + Send + Sync>,
}

impl<T: JobContextValues, R: Resolver> MultiCrawler<T, R> {
    pub fn new(pp_tx: Sender<ParserTask>, concurrency_profile: config::ConcurrencyProfile, networking_profile: config::NetworkingProfile) -> (MultiCrawler<T, R>, Sender<Job<T>>, Receiver<JobUpdate<T>>) {
        let (job_tx, job_rx) = bounded_ch::<Job<T>>(concurrency_profile.job_tx_buffer_size());
        let (update_tx, update_rx) = bounded_ch::<JobUpdate<T>>(concurrency_profile.job_update_buffer_size());
        (MultiCrawler {
            job_rx,
            update_tx,
            pp_tx,
            concurrency_profile,
            networking_profile,
            resolver: None,
        }, job_tx, update_rx)
    }

    pub fn set_name_resolver(&mut self, r: Arc<R>) {
        self.resolver = Some(r);
    }

    async fn process(&self) -> Result<()> {
        while let Ok(job) = self.job_rx.recv().await {
            let mut crawler = Crawler::new(job.url.clone(), job.settings.clone(), self.networking_profile.clone());
            if self.resolver.is_some() {
                crawler.set_name_resolver(self.resolver.clone().unwrap());
            }
            let _ = crawler.go(job.rules, self.pp_tx.clone(), self.update_tx.clone(), job.ctx)?.await;
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