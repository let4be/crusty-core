use ipnet::Ipv4Net;
use serde::{de, Deserialize, Deserializer};

use crate::{
	_prelude::*,
	resolver::{AsyncTrustDnsResolver, Resolver, RESERVED_V4_SUBNETS, RESERVED_V6_SUBNETS},
	types,
};

type Result<T> = types::Result<T>;

#[derive(Clone, Debug)]
pub struct CLevel(pub Level);

impl Deref for CLevel {
	type Target = Level;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'de> Deserialize<'de> for CLevel {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<CLevel, D::Error> {
		let s: String = Deserialize::deserialize(deserializer)?;
		Level::from_str(&s).map(CLevel).map_err(de::Error::custom)
	}
}

#[derive(Clone, Debug)]
pub struct CIP4Addr(pub Ipv4Addr);

impl Deref for CIP4Addr {
	type Target = Ipv4Addr;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'de> Deserialize<'de> for CIP4Addr {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<CIP4Addr, D::Error> {
		let s: String = Deserialize::deserialize(deserializer)?;
		let addr: Ipv4Addr = s.parse().map_err(de::Error::custom)?;
		Ok(CIP4Addr(addr))
	}
}

#[derive(Clone, Debug)]
pub struct CIP6Addr(pub Ipv6Addr);

impl Deref for CIP6Addr {
	type Target = Ipv6Addr;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'de> Deserialize<'de> for CIP6Addr {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<CIP6Addr, D::Error> {
		let s: String = Deserialize::deserialize(deserializer)?;
		let addr: Ipv6Addr = s.parse().map_err(de::Error::custom)?;
		Ok(CIP6Addr(addr))
	}
}

#[derive(Clone, Debug)]
pub struct CIpv4Net(pub Ipv4Net);

impl Deref for CIpv4Net {
	type Target = Ipv4Net;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'de> Deserialize<'de> for CIpv4Net {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<CIpv4Net, D::Error> {
		let s: String = Deserialize::deserialize(deserializer)?;
		s.parse::<Ipv4Net>().map(CIpv4Net).map_err(de::Error::custom)
	}
}

#[derive(Clone, Debug)]
pub struct CBytes(pub usize);

impl Deref for CBytes {
	type Target = usize;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl<'de> Deserialize<'de> for CBytes {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<CBytes, D::Error> {
		let s: String = Deserialize::deserialize(deserializer)?;
		let s = s.replace("_", "");
		let v = s.parse::<humanize_rs::bytes::Bytes>();
		let r = v.map_err(de::Error::custom)?;
		Ok(CBytes(r.size()))
	}
}

#[derive(Clone, Debug)]
pub struct CDuration(Duration);

impl Deref for CDuration {
	type Target = Duration;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

impl CDuration {
	pub fn from_secs(secs: u64) -> Self {
		CDuration(Duration::from_secs(secs))
	}

	pub fn from_millis(millis: u64) -> Self {
		CDuration(Duration::from_millis(millis))
	}
}

impl<'de> Deserialize<'de> for CDuration {
	fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<CDuration, D::Error> {
		let s: String = Deserialize::deserialize(deserializer)?;
		let s = s.replace("_", "");
		let v = humanize_rs::duration::parse(&s);
		let r = v.map_err(de::Error::custom)?;
		Ok(CDuration(r))
	}
}

pub type AsyncTrustDnsResolverConfig = trust_dns_resolver::config::ResolverConfig;
pub struct AsyncTrustDnsResolverOptsMapping(trust_dns_resolver::config::ResolverOpts);

impl Deref for AsyncTrustDnsResolverOptsMapping {
	type Target = trust_dns_resolver::config::ResolverOpts;

	fn deref(&self) -> &Self::Target {
		&self.0
	}
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AsyncTrustDnsResolverOpts {
	pub ndots:                  usize,
	pub timeout:                CDuration,
	pub attempts:               usize,
	pub rotate:                 bool,
	pub check_names:            bool,
	pub edns0:                  bool,
	pub validate:               bool,
	pub ip_strategy:            trust_dns_resolver::config::LookupIpStrategy,
	pub cache_size:             usize,
	pub use_hosts_file:         bool,
	pub positive_min_ttl:       Option<CDuration>,
	pub negative_min_ttl:       Option<CDuration>,
	pub positive_max_ttl:       Option<CDuration>,
	pub negative_max_ttl:       Option<CDuration>,
	pub num_concurrent_reqs:    usize,
	pub preserve_intermediates: bool,
}

impl From<AsyncTrustDnsResolverOpts> for AsyncTrustDnsResolverOptsMapping {
	fn from(p: AsyncTrustDnsResolverOpts) -> AsyncTrustDnsResolverOptsMapping {
		AsyncTrustDnsResolverOptsMapping(trust_dns_resolver::config::ResolverOpts {
			ndots:                  p.ndots,
			timeout:                *p.timeout,
			attempts:               p.attempts,
			rotate:                 p.rotate,
			check_names:            p.check_names,
			edns0:                  p.edns0,
			validate:               p.validate,
			ip_strategy:            p.ip_strategy,
			cache_size:             p.cache_size,
			use_hosts_file:         p.use_hosts_file,
			positive_min_ttl:       p.positive_min_ttl.map(|v| *v),
			negative_min_ttl:       p.negative_min_ttl.map(|v| *v),
			positive_max_ttl:       p.positive_max_ttl.map(|v| *v),
			negative_max_ttl:       p.negative_max_ttl.map(|v| *v),
			num_concurrent_reqs:    p.num_concurrent_reqs,
			preserve_intermediates: p.preserve_intermediates,
		})
	}
}

impl From<AsyncTrustDnsResolverOptsMapping> for AsyncTrustDnsResolverOpts {
	fn from(p: AsyncTrustDnsResolverOptsMapping) -> AsyncTrustDnsResolverOpts {
		AsyncTrustDnsResolverOpts {
			ndots:                  p.ndots,
			timeout:                CDuration(p.timeout),
			attempts:               p.attempts,
			rotate:                 p.rotate,
			check_names:            p.check_names,
			edns0:                  p.edns0,
			validate:               p.validate,
			ip_strategy:            p.ip_strategy,
			cache_size:             p.cache_size,
			use_hosts_file:         p.use_hosts_file,
			positive_min_ttl:       p.positive_min_ttl.map(CDuration),
			negative_min_ttl:       p.negative_min_ttl.map(CDuration),
			positive_max_ttl:       p.positive_max_ttl.map(CDuration),
			negative_max_ttl:       p.negative_max_ttl.map(CDuration),
			num_concurrent_reqs:    p.num_concurrent_reqs,
			preserve_intermediates: p.preserve_intermediates,
		}
	}
}

impl Default for AsyncTrustDnsResolverOpts {
	fn default() -> Self {
		AsyncTrustDnsResolverOptsMapping(trust_dns_resolver::config::ResolverOpts::default()).into()
	}
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConcurrencyProfile {
	pub parser_concurrency: usize,
	pub parser_pin:         usize,
	pub domain_concurrency: usize,
}

impl Default for ConcurrencyProfile {
	fn default() -> Self {
		let physical_cores = num_cpus::get_physical();
		Self { parser_concurrency: physical_cores, parser_pin: 0, domain_concurrency: physical_cores * 40 }
	}
}

impl ConcurrencyProfile {
	pub fn transit_buffer_size(&self) -> usize {
		self.domain_concurrency * 10
	}

	pub fn job_tx_buffer_size(&self) -> usize {
		self.domain_concurrency
	}

	pub fn job_update_buffer_size(&self) -> usize {
		self.domain_concurrency * 2
	}
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkingProfileValues {
	pub connect_timeout:          Option<CDuration>,
	pub socket_read_buffer_size:  Option<CBytes>,
	pub socket_write_buffer_size: Option<CBytes>,
	pub bind_local_ipv4:          Vec<CIP4Addr>,
	pub bind_local_ipv6:          Vec<CIP6Addr>,
}

impl Default for NetworkingProfileValues {
	fn default() -> Self {
		Self {
			connect_timeout:          Some(CDuration::from_secs(5)),
			socket_write_buffer_size: Some(CBytes(32 * 1024)),
			socket_read_buffer_size:  Some(CBytes(32 * 1024)),
			bind_local_ipv4:          vec![],
			bind_local_ipv6:          vec![],
		}
	}
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolverConfig {
	#[serde(default)]
	config:  Option<AsyncTrustDnsResolverConfig>,
	#[serde(default)]
	options: Option<AsyncTrustDnsResolverOpts>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkingProfile {
	pub values:           NetworkingProfileValues,
	pub resolver_config:  ResolverConfig,
	pub net_v4_blacklist: Vec<CIpv4Net>,

	#[serde(skip)]
	resolver: Option<Arc<Box<dyn Resolver>>>,
}

impl Default for NetworkingProfile {
	fn default() -> Self {
		Self {
			values:           NetworkingProfileValues::default(),
			resolver:         None,
			resolver_config:  ResolverConfig::default(),
			net_v4_blacklist: vec![],
		}
	}
}

impl NetworkingProfile {
	pub fn resolve(self) -> Result<ResolvedNetworkingProfile> {
		ResolvedNetworkingProfile::new(self)
	}
}

#[derive(Clone, Debug)]
pub struct ResolvedNetworkingProfile {
	pub values: NetworkingProfileValues,

	pub resolver: Arc<Box<dyn Resolver>>,
}

impl ResolvedNetworkingProfile {
	fn new(p: NetworkingProfile) -> Result<Self> {
		let values = p.values;

		if let Some(r) = p.resolver {
			return Ok(Self { values, resolver: r })
		}

		let (config, options) =
			if let (Some(config), Some(options)) = (p.resolver_config.config, p.resolver_config.options) {
				(config, options.into())
			} else {
				let (config, options) = trust_dns_resolver::system_conf::read_system_conf()
					.context("cannot read resolver config settings from system")?;
				(config, AsyncTrustDnsResolverOptsMapping(options))
			};

		let mut resolver = AsyncTrustDnsResolver::new(config, *options).context("cannot create default resolver")?;

		let reserved_v4 = RESERVED_V4_SUBNETS
			.clone()
			.into_iter()
			.chain(p.net_v4_blacklist.into_iter().map(|a| *a))
			.collect::<Vec<_>>();
		resolver.with_net_v4_blacklist(reserved_v4);
		resolver.with_net_v6_blacklist(RESERVED_V6_SUBNETS.to_vec());
		Ok(Self { values, resolver: Arc::new(Box::new(resolver)) })
	}
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrawlingSettings {
	pub internal_read_buffer_size: CBytes,
	pub concurrency:               usize,
	pub max_response_size:         CBytes,
	pub delay:                     CDuration,
	pub delay_jitter:              CDuration,
	pub status_timeout:            CDuration,
	pub load_timeout:              CDuration,
	pub job_soft_timeout:          CDuration,
	pub job_hard_timeout:          CDuration,
	pub job_hard_timeout_jitter:   CDuration,
	pub custom_headers:            HashMap<String, Vec<String>>,
	pub user_agent:                Option<String>,
	pub compression:               bool,
}

impl fmt::Display for CrawlingSettings {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"concurrency: {}, delay: {:?}, job hard timeout: {:?}+~{:?}, job soft timeout: {:?}, irbs: {:?}, load timeout: {:?}, max_response_size: {:?}, custom headers: {:?}",
			self.concurrency, self.delay, self.job_hard_timeout,  self.job_hard_timeout_jitter, self.job_soft_timeout, self.internal_read_buffer_size, self.load_timeout, self.max_response_size, self.custom_headers,
		)
	}
}

impl Default for CrawlingSettings {
	fn default() -> Self {
		Self {
			concurrency:               2,
			internal_read_buffer_size: CBytes(32 * 1024),
			delay:                     CDuration::from_secs(1),
			delay_jitter:              CDuration::from_secs(1),
			job_hard_timeout:          CDuration::from_secs(360),
			job_hard_timeout_jitter:   CDuration::from_secs(72),
			job_soft_timeout:          CDuration::from_secs(240),
			status_timeout:            CDuration::from_secs(5),
			load_timeout:              CDuration::from_secs(10),
			user_agent:                Some(String::from("crusty-core/0.67.1")),
			compression:               true,
			custom_headers:            HashMap::new(),
			max_response_size:         CBytes(1024 * 1024 * 2),
		}
		.build_headers()
	}
}

impl CrawlingSettings {
	pub fn build_headers(mut self) -> Self {
		if let Some(user_agent) = &self.user_agent {
			self.custom_headers.insert(http::header::USER_AGENT.to_string(), vec![user_agent.clone()]);
		}
		if self.compression {
			self.custom_headers.insert(http::header::ACCEPT_ENCODING.to_string(), vec!["gzip, deflate".into()]);
		}
		self.custom_headers.insert(
			http::header::ACCEPT.to_string(),
			vec!["text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9".into()],
		);
		self
	}
}
