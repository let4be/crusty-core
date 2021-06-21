#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{
	config,
	config::ResolvedNetworkingProfile,
	hyper_utils, load_filters,
	parser_processor::ParserProcessor,
	resolver::{Adaptor, AsyncStaticResolver},
	status_filters, task_expanders, task_filters,
	task_processor::*,
	task_scheduler::*,
	types::*,
};

#[derive(Clone)]
pub struct CrawlingRulesOptions {
	pub max_redirect:           usize,
	pub allow_www:              bool,
	pub page_budget:            Option<usize>,
	pub links_per_page_budget:  Option<usize>,
	pub max_level:              Option<usize>,
	pub accepted_content_types: Option<Vec<&'static str>>,
	pub robots_txt:             bool,
}

impl Default for CrawlingRulesOptions {
	fn default() -> Self {
		Self {
			max_redirect:           5,
			allow_www:              true,
			page_budget:            Some(50),
			links_per_page_budget:  Some(50),
			max_level:              Some(10),
			accepted_content_types: Some(vec!["text/html", "text/plain"]),
			robots_txt:             true,
		}
	}
}

pub struct CrawlingRules<JS, TS, P: ParsedDocument> {
	pub options: CrawlingRulesOptions,

	pub custom_status_filters:  Vec<BoxedStatusFilter<JS, TS>>,
	pub custom_load_filter:     Vec<BoxedLoadFilter<JS, TS>>,
	pub custom_task_filter:     Vec<BoxedTaskFilter<JS, TS>>,
	pub custom_task_expanders:  Vec<BoxedTaskExpander<JS, TS, P>>,
	pub custom_document_parser: Arc<DocumentParser<P>>,
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> CrawlingRules<JS, TS, P> {
	pub fn new(opts: CrawlingRulesOptions, custom_document_parser: DocumentParser<P>) -> Self {
		Self {
			options: opts,

			custom_status_filters:  vec![],
			custom_load_filter:     vec![],
			custom_task_filter:     vec![],
			custom_task_expanders:  vec![],
			custom_document_parser: Arc::new(custom_document_parser),
		}
	}

	pub fn with_status_filter<
		H: status_filters::Filter<JS, TS> + Send + Sync + 'static,
		T: Fn() -> H + Send + Sync + 'static,
	>(
		mut self,
		f: T,
	) -> Self {
		self.custom_status_filters.push(Box::new(move || Box::new(f())));
		self
	}

	pub fn with_load_filter<
		H: load_filters::Filter<JS, TS> + Send + Sync + 'static,
		T: Fn() -> H + Send + Sync + 'static,
	>(
		mut self,
		f: T,
	) -> Self {
		self.custom_load_filter.push(Box::new(move || Box::new(f())));
		self
	}

	pub fn with_task_filter<
		H: task_filters::Filter<JS, TS> + Send + Sync + 'static,
		T: Fn() -> H + Send + Sync + 'static,
	>(
		mut self,
		f: T,
	) -> Self {
		self.custom_task_filter.push(Box::new(move || Box::new(f())));
		self
	}

	pub fn with_task_expander<
		H: task_expanders::Expander<JS, TS, P> + Send + Sync + 'static,
		T: Fn() -> H + Send + Sync + 'static,
	>(
		mut self,
		f: T,
	) -> Self {
		self.custom_task_expanders.push(Box::new(move || Box::new(f())));
		self
	}
}

impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> JobRules<JS, TS, P> for CrawlingRules<JS, TS, P> {
	fn task_filters(&self) -> TaskFilters<JS, TS> {
		let options = &self.options;
		let mut task_filters: TaskFilters<JS, TS> = vec![
			Box::new(task_filters::MaxRedirect::new(options.max_redirect)),
			Box::new(task_filters::SkipNoFollowLinks::new()),
			Box::new(task_filters::SelectiveTaskFilter::new(
				// this filter is enabled only for links that we follow(either directly with get or with head+get)
				vec![LinkTarget::Follow, LinkTarget::HeadFollow],
				task_filters::SameDomain::new(options.allow_www),
			)),
		];

		let dedup_checking = task_filters::HashSetDedup::new(true);
		let dedup_committing = dedup_checking.committing();
		task_filters.push(Box::new(dedup_checking));

		if let Some(page_budget) = options.page_budget {
			task_filters.push(Box::new(task_filters::TotalPageBudget::new(page_budget)))
		}
		if let Some(links_per_page_budget) = options.links_per_page_budget {
			task_filters.push(Box::new(task_filters::LinkPerPageBudget::new(links_per_page_budget)))
		}
		if let Some(max_level) = options.max_level {
			task_filters.push(Box::new(task_filters::PageLevel::new(max_level)))
		}

		if options.robots_txt {
			task_filters.push(Box::new(task_filters::RobotsTxt::new()))
		}

		for e in &self.custom_task_filter {
			task_filters.push(e());
		}

		task_filters.push(Box::new(dedup_committing));
		task_filters
	}

	fn status_filters(&self) -> StatusFilters<JS, TS> {
		let options = &self.options;

		let mut status_filters: StatusFilters<JS, TS> = vec![Box::new(status_filters::Redirect::new())];
		if let Some(v) = &options.accepted_content_types {
			status_filters.push(Box::new(status_filters::ContentType::new(v.clone())));
		}

		for e in &self.custom_status_filters {
			status_filters.push(e());
		}
		status_filters
	}

	fn load_filters(&self) -> LoadFilters<JS, TS> {
		let options = &self.options;
		let mut load_filters = vec![];

		for e in &self.custom_load_filter {
			load_filters.push(e());
		}

		if options.robots_txt {
			load_filters.push(Box::new(load_filters::RobotsTxt::new()))
		}

		load_filters
	}

	fn task_expanders(&self) -> TaskExpanders<JS, TS, P> {
		let mut task_expanders: TaskExpanders<JS, TS, P> = vec![];

		for e in &self.custom_task_expanders {
			task_expanders.push(e());
		}
		task_expanders
	}

	fn document_parser(&self) -> Arc<DocumentParser<P>> {
		Arc::clone(&self.custom_document_parser)
	}
}

#[derive(Clone)]
struct HttpClientFactory {
	networking_profile: config::ResolvedNetworkingProfile,
}

impl ClientFactory for HttpClientFactory {
	fn make(&self) -> (HttpClient, hyper_utils::Stats) {
		let v = self.networking_profile.values.clone();

		let resolver_adaptor = Adaptor::new(Arc::clone(&self.networking_profile.resolver));
		let mut http = hyper::client::HttpConnector::new_with_resolver(resolver_adaptor);
		http.set_connect_timeout(v.connect_timeout.map(|v| *v));

		let bind_local_ipv4 = if !v.bind_local_ipv4.is_empty() {
			if v.bind_local_ipv4.len() == 1 {
				Some(*v.bind_local_ipv4[0])
			} else {
				let mut rng = thread_rng();
				Some(*v.bind_local_ipv4[rng.gen_range(0..v.bind_local_ipv4.len())])
			}
		} else {
			None
		};

		let bind_local_ipv6 = if !v.bind_local_ipv6.is_empty() {
			if v.bind_local_ipv6.len() == 1 {
				Some(*v.bind_local_ipv6[0])
			} else {
				let mut rng = thread_rng();
				Some(*v.bind_local_ipv6[rng.gen_range(0..v.bind_local_ipv6.len())])
			}
		} else {
			None
		};

		match (bind_local_ipv4, bind_local_ipv6) {
			(Some(ipv4), Some(ipv6)) => http.set_local_addresses(ipv4, ipv6),
			(Some(ipv4), None) => http.set_local_address(Some(IpAddr::V4(ipv4))),
			(None, Some(ipv6)) => http.set_local_address(Some(IpAddr::V6(ipv6))),
			(None, None) => {}
		}

		http.set_recv_buffer_size(v.socket_read_buffer_size.map(|v| *v));
		http.set_send_buffer_size(v.socket_write_buffer_size.map(|v| *v));
		http.enforce_http(false);
		let http_counting = hyper_utils::CountingConnector::new(http);
		let stats = http_counting.stats.clone();
		let https = hyper_tls::HttpsConnector::new_with_connector(http_counting);
		let client = hyper::Client::builder().build::<_, hyper::Body>(https);
		(client, stats)
	}
}

pub struct Crawler {
	networking_profile: config::ResolvedNetworkingProfile,
	parse_tx:           Sender<ParserTask>,
}

pub struct CrawlerIter<JS: JobStateValues, TS: TaskStateValues> {
	rx: Receiver<JobUpdate<JS, TS>>,
	_h: tokio::task::JoinHandle<Result<()>>,
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

impl Crawler {
	pub fn new_default() -> anyhow::Result<Crawler> {
		let concurrency_profile = config::ConcurrencyProfile::default();
		let tx_pp = ParserProcessor::spawn(concurrency_profile, 1024 * 1024 * 32);

		let networking_profile = config::NetworkingProfile::default().resolve()?;
		Ok(Crawler::new(networking_profile, tx_pp))
	}
}

impl Crawler {
	pub fn new(networking_profile: config::ResolvedNetworkingProfile, tx_pp: Sender<ParserTask>) -> Crawler {
		Crawler { networking_profile, parse_tx: tx_pp }
	}

	pub fn iter<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument>(
		self,
		job: Job<JS, TS, P>,
	) -> CrawlerIter<JS, TS> {
		let (tx, rx) = unbounded_ch();

		let h = tokio::spawn(async move {
			self.go(job, tx).await?;
			Ok::<_, Error>(())
		});

		CrawlerIter { rx, _h: h }
	}

	pub fn go<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument>(
		&self,
		job: Job<JS, TS, P>,
		update_tx: Sender<JobUpdate<JS, TS>>,
	) -> PinnedTask {
		TracingTask::new(span!(url=%job.url), async move {
			let job = ResolvedJob::from(job);

			let scheduler = TaskScheduler::new(job.clone(), update_tx.clone());

			let client_factory = HttpClientFactory { networking_profile: self.networking_profile.clone() };

			let mut processor_handles = vec![];
			for i in 0..job.settings.concurrency {
				let processor = TaskProcessor::new(
					job.clone(),
					scheduler.job_update_tx.clone(),
					scheduler.tasks_rx.clone(),
					self.parse_tx.clone(),
					Box::new(client_factory.clone()),
					Arc::clone(&self.networking_profile.resolver),
					true,
				);

				processor_handles.push(tokio::spawn(async move { processor.go(i).await }));
			}

			let job_finishing_update = scheduler.go()?.await?;

			trace!("Waiting for workers to finish...");
			for h in processor_handles {
				let _ = h.await?;
			}
			trace!("Workers are done - sending notification about job done!");

			let _ = update_tx.send_async(job_finishing_update).await;
			Ok(())
		})
		.instrument()
	}
}

#[derive(Derivative)]
#[derivative(Clone(bound = ""))]
pub struct MultiCrawler<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> {
	job_rx:              Receiver<Job<JS, TS, P>>,
	update_tx:           Sender<JobUpdate<JS, TS>>,
	parse_tx:            Sender<ParserTask>,
	concurrency_profile: config::ConcurrencyProfile,
	networking_profile:  config::ResolvedNetworkingProfile,
}

pub type MultiCrawlerTuple<JS, TS, P> = (MultiCrawler<JS, TS, P>, Sender<Job<JS, TS, P>>, Receiver<JobUpdate<JS, TS>>);
impl<JS: JobStateValues, TS: TaskStateValues, P: ParsedDocument> MultiCrawler<JS, TS, P> {
	pub fn new(
		tx_pp: Sender<ParserTask>,
		concurrency_profile: config::ConcurrencyProfile,
		networking_profile: config::ResolvedNetworkingProfile,
	) -> MultiCrawlerTuple<JS, TS, P> {
		let (job_tx, job_rx) = bounded_ch::<Job<JS, TS, P>>(concurrency_profile.job_tx_buffer_size());
		let (update_tx, update_rx) = bounded_ch::<JobUpdate<JS, TS>>(concurrency_profile.job_update_buffer_size());
		(
			MultiCrawler { job_rx, update_tx, parse_tx: tx_pp, concurrency_profile, networking_profile },
			job_tx,
			update_rx,
		)
	}

	async fn process(self) -> Result<()> {
		while let Ok(job) = self.job_rx.recv_async().await {
			let np = self.networking_profile.clone();

			if job.addrs.is_some() {
				let np_static = ResolvedNetworkingProfile {
					values:   np.values,
					resolver: Arc::new(Box::new(AsyncStaticResolver::new(job.addrs.clone().unwrap().clone()))),
				};

				let crawler = Crawler::new(np_static, self.parse_tx.clone());
				let _ = crawler.go(job, self.update_tx.clone()).await;
			} else {
				let crawler = Crawler::new(np, self.parse_tx.clone());
				let _ = crawler.go(job, self.update_tx.clone()).await;
			}
		}
		Ok(())
	}

	pub fn go(self) -> PinnedTask<'static> {
		TracingTask::new(span!(), async move {
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
		})
		.instrument()
	}
}
