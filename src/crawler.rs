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
    resolver::AsyncHyperResolver
};

use std::{
    sync::{Arc},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use futures::{future};
use url::Url;
use anyhow::{anyhow};


#[derive(Clone, Debug)]
pub struct Crawler {
    url: Url,
    settings: config::CrawlerSettings,
    networking_profile: config::NetworkingProfile
}

#[derive(Clone)]
struct HttpClientFactory {
    settings: config::CrawlerSettings,
    addr_ipv4: Option<Ipv4Addr>,
    addr_ipv6: Option<Ipv6Addr>,
    resolver: AsyncHyperResolver,
}

type HttpClient = hyper::Client<HttpConnector>;
type HttpConnector = hyper_tls::HttpsConnector<hyper_utils::CountingConnector<hyper::client::HttpConnector<AsyncHyperResolver>>>;

impl HttpClientFactory {
    fn make(&self) -> (HttpClient, hyper_utils::Stats) {
        let mut http = hyper::client::HttpConnector::new_with_resolver(self.resolver.clone());
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

impl Crawler {
    pub fn new(url: Url, settings: config::CrawlerSettings, networking_profile: config::NetworkingProfile) -> Crawler {
        Crawler {
            url,
            settings,
            networking_profile
        }
    }

    pub fn go<T: JobContextValues>(
        &self,
        rules: Box<dyn JobRules<T> + Send + Sync>,
        parse_tx: Sender<ParserTask>,
        sub_tx: Sender<JobUpdate<T>>,
        ctx: StdJobContext<T>,
        resolver: AsyncHyperResolver
    ) -> PinnedFut
    {
        TracingTask::new(span!(Level::INFO, url=self.url.as_str()), async move {
            let mut scheduler = TaskScheduler::new(
                &self.url,
                &self.settings,
                rules.task_filters(),
                ctx.clone(),
                sub_tx.clone()
            )?;

            let client_factory = HttpClientFactory {
                settings: self.settings.clone(),
                resolver: resolver.clone(),
                addr_ipv4: self.networking_profile.bind_local_ipv4.clone().map(|v|*v),
                addr_ipv6: self.networking_profile.bind_local_ipv6.clone().map(|v|*v),
            };

            let mut processor_handles = vec![];
            for i in 0..self.settings.concurrency {
                let client_factory = (|cf: HttpClientFactory|
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
        }).instrument()
    }
}

#[derive(Clone)]
pub struct MultiCrawler<T: JobContextValues> {
    job_rx: Receiver<Job<T>>,
    update_tx: Sender<JobUpdate<T>>,
    pp_tx: Sender<ParserTask>,
    concurrency_profile: config::ConcurrencyProfile,
    networking_profile: config::NetworkingProfile
}

pub struct Job<T: JobContextValues> {
    pub ctx: StdJobContext<T>,
    pub url: url::Url,
    pub settings: config::CrawlerSettings,
    pub rules: Box<dyn JobRules<T> + Send + Sync>,
}

pub trait JobRules<T: JobContextValues> {
    fn task_filters(&self) -> Vec<Box<dyn task_filters::TaskFilter<T> + Send + Sync>>;
    fn status_filters(&self) -> Vec<Box<dyn status_filters::StatusFilter<T> + Send + Sync>>;
    fn load_filters(&self) -> Vec<Box<dyn load_filters::LoadFilter<T> + Send + Sync>>;
    fn task_expanders(&self) -> Vec<Box<dyn expanders::TaskExpander<T> + Send + Sync>>;
}

impl<T: JobContextValues> MultiCrawler<T> {
    pub fn new(pp_tx: Sender<ParserTask>, concurrency_profile: config::ConcurrencyProfile, networking_profile: config::NetworkingProfile) -> (MultiCrawler<T>, Sender<Job<T>>, Receiver<JobUpdate<T>>) {
        let (job_tx, job_rx) = bounded_ch::<Job<T>>(concurrency_profile.job_tx_buffer_size());
        let (update_tx, update_rx) = bounded_ch::<JobUpdate<T>>(concurrency_profile.job_update_buffer_size());
        (MultiCrawler {
            job_rx,
            update_tx,
            pp_tx,
            concurrency_profile,
            networking_profile
        }, job_tx, update_rx)
    }

    async fn process(&self, resolver: AsyncHyperResolver) {
        while let Ok(job) = self.job_rx.recv().await {
            let crawler = Crawler::new(job.url.clone(), job.settings.clone(), self.networking_profile.clone());
            let _ = crawler.go(job.rules, self.pp_tx.clone(), self.update_tx.clone(), job.ctx, resolver.clone()).await;
        }
    }

    pub fn go(self) -> PinnedFut<'static> {
        TracingTask::new(span!(Level::INFO), async move {
            let resolver = AsyncHyperResolver::new_from_system_conf();
            if resolver.is_err() {
                return Err(anyhow!("Cannot initialize async dns resolver: {}", resolver.err().unwrap()).into());
            }
            let resolver = resolver.unwrap();

            let mut handles = vec![];
            let s = Arc::new(self);

            for _ in 0..s.concurrency_profile.domain_concurrency {
                let h = tokio::spawn({
                    let mc = s.clone();
                    let resolver = resolver.clone();
                    async move {
                        mc.process(resolver).await;
                    }
                });
                handles.push(h);
            }

            let _ = future::join_all(handles).await;
            Ok(())
        }).instrument()
    }
}