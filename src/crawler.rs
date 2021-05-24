#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{
    types::*,
    parser_processor::ParserProcessor,
    parser_processor::Handle,
    task_scheduler::*,
    task_processor::*,
    config,
    task_filters,
    hyper_utils,
    status_filters,
    load_filters,
    task_expanders,
    resolver::Adaptor,
    resolver::AsyncHyperResolver,
    resolver::Resolver
};

use std::{
    net::{IpAddr},
};

#[derive(Clone)]
pub struct CrawlingRulesOptions {
    pub max_redirect: usize,
    pub link_target: LinkTarget,
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
            max_redirect: 5,
            link_target: LinkTarget::HeadFollow,
            allow_www: true,
            page_budget: Some(50),
            links_per_page_budget: Some(50),
            max_level: Some(10),
            accepted_content_types: Some(vec![String::from("text/html")]),
            redirect_term_on_err: Some(true),
        }
    }
}

pub struct CrawlingRules<JS, TS> {
    pub options: CrawlingRulesOptions,

    pub custom_status_filters: Vec<BoxedStatusFilter<JS, TS>>,
    pub custom_load_filter: Vec<BoxedLoadFilter<JS, TS>>,
    pub custom_task_filter: Vec<BoxedTaskFilter<JS, TS>>,
    pub custom_task_expanders: Vec<BoxedTaskExpander<JS, TS>>,
}

impl<JS: JobStateValues, TS: TaskStateValues> Default for CrawlingRules<JS, TS>{
    fn default() -> Self {
        Self::new(CrawlingRulesOptions::default())
    }
}

impl<JS: JobStateValues, TS: TaskStateValues> CrawlingRules<JS, TS> {
    pub fn new(opts: CrawlingRulesOptions) -> Self {
        Self {
            options: opts,

            custom_status_filters: vec![],
            custom_load_filter: vec![],
            custom_task_filter: vec![],
            custom_task_expanders: vec![],
        }
    }

    pub fn with_status_filter<H: status_filters::Filter<JS, TS> + Send + Sync + 'static, T: Fn() -> H + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_status_filters.push(
            Box::new(move ||
                Box::new(f())
            )
        );
        self
    }

    pub fn with_load_filter<H: load_filters::Filter<JS, TS> + Send + Sync + 'static, T: Fn() -> H + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_load_filter.push(
            Box::new(move ||
                Box::new(f())
            )
        );
        self
    }

    pub fn with_task_filter<H: task_filters::Filter<JS, TS> + Send + Sync + 'static, T: Fn() -> H + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_task_filter.push(
            Box::new(move ||
                Box::new(f())
            )
        );
        self
    }

    pub fn with_task_expander<H: task_expanders::Expander<JS, TS> + Send + Sync + 'static, T: Fn() -> H + Send + Sync + 'static>(mut self, f: T) -> Self {
        self.custom_task_expanders.push(
            Box::new(move ||
                Box::new(f())
            )
        );
        self
    }
}

impl<JS: JobStateValues, TS: TaskStateValues> JobRules<JS, TS> for CrawlingRules<JS, TS> {
    fn task_filters(&self) -> TaskFilters<JS, TS> {
        let options = &self.options;
        let mut task_filters: TaskFilters<JS, TS> = vec![
            Box::new(task_filters::MaxRedirect::new(options.max_redirect)),
            Box::new(task_filters::SameDomain::new(options.allow_www)),
            Box::new(task_filters::HashSetDedup::new()),
        ];
        if options.page_budget.is_some() {
            task_filters.push(Box::new(task_filters::TotalPageBudget::new(options.page_budget.unwrap())));
        }
        if options.links_per_page_budget.is_some() {
            task_filters.push(Box::new(task_filters::LinkPerPageBudget::new(options.links_per_page_budget.unwrap())))
        }
        if options.max_level.is_some() {
            task_filters.push(Box::new(task_filters::PageLevel::new(options.max_level.unwrap())))
        }

        for e in &self.custom_task_filter {
            task_filters.push(e());
        }
        task_filters
    }

    fn status_filters(&self) -> StatusFilters<JS, TS> {
        let options = &self.options;

        let mut status_filters: StatusFilters<JS, TS> = vec![];
        if options.accepted_content_types.is_some() {
            status_filters.push(Box::new(status_filters::ContentType::new(options.accepted_content_types.clone().unwrap(), true)));
        }
        if options.redirect_term_on_err.is_some() {
            status_filters.push(Box::new(status_filters::Redirect::new(true)));
        }

        for e in &self.custom_status_filters {
            status_filters.push(e());
        }
        status_filters
    }

    fn load_filters(&self) -> LoadFilters<JS, TS> {
        let mut load_filters = vec![];

        for e in &self.custom_load_filter {
            load_filters.push(e());
        }
        load_filters
    }

    fn task_expanders(&self) -> TaskExpanders<JS, TS> {
        let mut task_expanders : TaskExpanders<JS, TS> = vec![
            Box::new(task_expanders::FollowLinks::new(self.options.link_target.clone())),
        ];

        for e in &self.custom_task_expanders {
            task_expanders.push(e());
        }
        task_expanders
    }
}

#[derive(Clone)]
struct HttpClientFactory<R: Resolver> {
    networking_profile: config::ResolvedNetworkingProfile<R>,
}

type HttpClient<R> = hyper::Client<HttpConnector<R>>;
type HttpConnector<R> = hyper_tls::HttpsConnector<hyper_utils::CountingConnector<hyper::client::HttpConnector<Adaptor<R>>>>;

impl<R: Resolver> HttpClientFactory<R> {
    fn make(&self) -> (HttpClient<R>, hyper_utils::Stats) {
        let v = self.networking_profile.values.clone();

        let resolver_adaptor = Adaptor::new(Arc::clone(&self.networking_profile.resolver));
        let mut http = hyper::client::HttpConnector::new_with_resolver(resolver_adaptor);
        http.set_connect_timeout(v.connect_timeout.clone().map(|v| *v));

        match (v.bind_local_ipv4.map(|v|*v), v.bind_local_ipv6.map(|v|*v)) {
            (Some(ipv4), Some(ipv6)) => http.set_local_addresses(ipv4, ipv6),
            (Some(ipv4), None) => http.set_local_address(Some(IpAddr::V4(ipv4))),
            (None, Some(ipv6)) => http.set_local_address(Some(IpAddr::V6(ipv6))),
            (None, None) => {}
        }

        http.set_recv_buffer_size(v.socket_read_buffer_size.map(|v|*v));
        http.set_send_buffer_size(v.socket_write_buffer_size.map(|v|*v));
        http.enforce_http(false);
        let http_counting = hyper_utils::CountingConnector::new(http);
        let stats = http_counting.stats.clone();
        let https = hyper_tls::HttpsConnector::new_with_connector(http_counting);
        let client = hyper::Client::builder().build::<_, hyper::Body>(https);

        (client, stats)
    }
}

pub struct Crawler<R: Resolver = AsyncHyperResolver>{
    networking_profile: config::ResolvedNetworkingProfile<R>,
    parse_tx: Sender<ParserTask>,
}

pub struct CrawlerIter<JS: JobStateValues, TS: TaskStateValues>{
    rx: Receiver<JobUpdate<JS, TS>>,
    _h: tokio::task::JoinHandle<Result<()>>
}

impl<JS: JobStateValues, TS: TaskStateValues> Iterator for CrawlerIter<JS, TS> {
    type Item = JobUpdate<JS, TS>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rx.recv().ok()
    }
}

impl<JS: JobStateValues, TS: TaskStateValues> CrawlerIter<JS, TS> {
    pub fn join(self) -> tokio::task::JoinHandle<Result<()>> {
        self._h
    }
}

impl Crawler<AsyncHyperResolver> {
    pub fn new_default() -> anyhow::Result<Crawler<AsyncHyperResolver>> {
        let concurrency_profile = config::ConcurrencyProfile::default();
        let pp = ParserProcessor::spawn(concurrency_profile, 1024 * 1024 * 32);

        let networking_profile = config::NetworkingProfile::default().resolve()?;
        Ok(Crawler::new(networking_profile, &pp))
    }
}

impl<R: Resolver> Crawler<R> {
    pub fn new(networking_profile: config::ResolvedNetworkingProfile<R>, pp: &Handle) -> Crawler<R> {
        Self::_new(networking_profile, pp.tx.clone())
    }

    pub fn _new(networking_profile: config::ResolvedNetworkingProfile<R>, parse_tx: Sender<ParserTask>) -> Crawler<R> {
        Crawler {
            networking_profile,
            parse_tx,
        }
    }

    pub fn iter<JS: JobStateValues, TS: TaskStateValues>(self, job: Job<JS, TS>) -> CrawlerIter<JS, TS> {
        let (tx, rx) = unbounded_ch();

        let h = tokio::spawn(async move {
            self.go(job,tx).await?;
            Ok::<_, Error>(())
        });

        CrawlerIter{rx, _h: h}
    }

    pub fn go<JS: JobStateValues, TS: TaskStateValues>(
        &self,
        job: Job<JS, TS>,
        update_tx: Sender<JobUpdate<JS, TS>>,
    ) -> PinnedTask
    {
        TracingTask::new(span!(Level::INFO, url=job.url.as_str()), async move {
            let job = ResolvedJob::from(job);

            let scheduler = TaskScheduler::new(
                job.clone(),
                update_tx.clone()
            );

            let client_factory = HttpClientFactory {
                networking_profile: self.networking_profile.clone(),
            };

            let mut processor_handles = vec![];
            for i in 0..job.settings.concurrency {
                let cf = client_factory.clone();
                let mut processor = TaskProcessor::new(
                    job.clone(),
                    scheduler.job_update_tx.clone(),
                    scheduler.tasks_rx.clone(),
                    self.parse_tx.clone(),
                    Box::new(move || cf.make())
                );

                processor_handles.push(tokio::spawn(async move {
                    processor.go(i).await
                }));
            }

            let job_finishing_update = scheduler.go()?.await?;

            trace!("Waiting for workers to finish...");
            for h in processor_handles {
                let _ = h.await?;
            }
            trace!("Workers are done - sending notification about job done!");

            let _ = update_tx.send_async(job_finishing_update).await;
            Ok(())
        }).instrument()
    }
}

#[derive(Clone)]
pub struct MultiCrawler<JS: JobStateValues, TS: TaskStateValues, R: Resolver> {
    job_rx: Receiver<Job<JS, TS>>,
    update_tx: Sender<JobUpdate<JS, TS>>,
    parse_tx: Sender<ParserTask>,
    concurrency_profile: config::ConcurrencyProfile,
    networking_profile: config::ResolvedNetworkingProfile<R>,
}

pub type MultiCrawlerTuple<JS, TS, R> = (MultiCrawler<JS, TS, R>, Sender<Job<JS, TS>>, Receiver<JobUpdate<JS, TS>>);
impl<JS: JobStateValues, TS: TaskStateValues, R: Resolver> MultiCrawler<JS, TS, R> {

    pub fn new(pp_h: &Handle, concurrency_profile: config::ConcurrencyProfile, networking_profile: config::ResolvedNetworkingProfile<R>) -> MultiCrawlerTuple<JS, TS, R> {
        let (job_tx, job_rx) = bounded_ch::<Job<JS, TS>>(concurrency_profile.job_tx_buffer_size());
        let (update_tx, update_rx) = bounded_ch::<JobUpdate<JS, TS>>(concurrency_profile.job_update_buffer_size());
        (MultiCrawler {
            job_rx,
            update_tx,
            parse_tx: pp_h.tx.clone(),
            concurrency_profile,
            networking_profile,
        }, job_tx, update_rx)
    }

    async fn process(self) -> Result<()> {
        while let Ok(job) = self.job_rx.recv_async().await {
            let crawler = Crawler::_new( self.networking_profile.clone(), self.parse_tx.clone());
            let _ = crawler.go(job, self.update_tx.clone()).await;
        }
        Ok(())
    }

    pub fn go(self) -> PinnedTask<'static> {
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

            for h in handles {
                let _ = h.await?;
            }
            Ok(())
        }).instrument()
    }
}