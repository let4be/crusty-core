#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    types,
    resolver::AsyncHyperResolver,
    resolver::Resolver
};

use std::{
    fmt, collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
    ops::Deref,
    str::FromStr,
    sync::Arc
};

use serde::{Deserialize, Serialize, Deserializer, de};

pub type Result<T> = types::Result<T>;

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
        Level::from_str(&s).map(|e|CLevel(e)).map_err(de::Error::custom)
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
        let s = s.replace("_","");
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
    pub fn from_millis(millis: u64) -> Self { CDuration(Duration::from_millis(millis)) }
}

impl<'de> Deserialize<'de> for CDuration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<CDuration, D::Error> {
        let s: String = Deserialize::deserialize(deserializer)?;
        let s = s.replace("_","");
        let v = humanize_rs::duration::parse(&s);
        let r = v.map_err(de::Error::custom)?;
        Ok(CDuration(r))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConcurrencyProfile {
    pub parser_concurrency: usize,
    pub domain_concurrency: usize,
}

impl Default for ConcurrencyProfile {
    fn default() -> Self {
        let physical_cores = num_cpus::get_physical();
        Self {
            parser_concurrency: physical_cores,
            domain_concurrency: physical_cores * 30,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct NetworkingProfile<R: Resolver = AsyncHyperResolver> {
    pub bind_local_ipv4: Option<CIP4Addr>,
    pub bind_local_ipv6: Option<CIP6Addr>,

    #[serde(skip)]
    pub resolver: Option<Arc<R>>
}

impl<R: Resolver> NetworkingProfile<R> {
    pub fn resolve(self) -> Result<ResolvedNetworkingProfile<R>> {
        ResolvedNetworkingProfile::new(self)
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedNetworkingProfile<R: Resolver = AsyncHyperResolver> {
    pub bind_local_ipv4: Option<CIP4Addr>,
    pub bind_local_ipv6: Option<CIP6Addr>,

    pub resolver: Arc<R>
}

impl Default for NetworkingProfile {
    fn default() -> Self {
        Self{bind_local_ipv4: None, bind_local_ipv6: None, resolver: None}
    }
}

impl<R: Resolver> ResolvedNetworkingProfile<R> {
    fn new(p: NetworkingProfile<R>) -> Result<Self> {
        let mut resolver = p.resolver;
        if resolver.is_none() {
            resolver = Some(Arc::new(Resolver::new_default().context("cannot create default resolver")?));
        }
        let resolver = resolver.unwrap();
        Ok(Self {
            bind_local_ipv6: p.bind_local_ipv6,
            bind_local_ipv4: p.bind_local_ipv4,
            resolver
        })
    }
}

impl ConcurrencyProfile {
    pub fn transit_buffer_size(&self) -> usize {
        self.domain_concurrency * 10
    }

    pub fn job_tx_buffer_size(&self) -> usize
    {
        self.domain_concurrency * 3
    }

    pub fn job_update_buffer_size(&self) -> usize {
        self.domain_concurrency * 3
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct CrawlerSettings {
    pub concurrency: usize,
    pub internal_read_buffer_size: CBytes,
    pub socket_read_buffer_size: Option<CBytes>,
    pub socket_write_buffer_size: Option<CBytes>,
    pub max_response_size: CBytes,
    pub max_redirect: usize,
    pub delay: CDuration,
    pub connect_timeout: Option<CDuration>,
    pub load_timeout: CDuration,
    pub job_soft_timeout: CDuration,
    pub job_hard_timeout: CDuration,
    pub custom_headers: HashMap<String, Vec<String>>,
}

impl fmt::Display for CrawlerSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "concurrency: {}, delay: {:?}, job hard timeout: {:?}, job soft timeout: {:?}, connect timeout: {:?}, irbs: {:?}, socker rbs: {:?}, socket wbs: {:?}, load timeout: {:?}, max_response_size: {:?}, max_redirect: {:?}, custom headers: {:?}",
            self.concurrency,
            self.delay,
            self.job_hard_timeout,
            self.job_soft_timeout,
            self.connect_timeout,
            self.internal_read_buffer_size,
            self.socket_read_buffer_size,
            self.socket_write_buffer_size,
            self.load_timeout,
            self.max_response_size,
            self.max_redirect,
            self.custom_headers,
        )
    }
}

impl Default for CrawlerSettings {
    fn default() -> Self {
        Self {
            concurrency: 2,
            delay: CDuration::from_secs(1),
            job_hard_timeout: CDuration::from_secs(60),
            job_soft_timeout: CDuration::from_secs(30),
            load_timeout: CDuration::from_secs(10),
            connect_timeout: Some(CDuration::from_secs(5)),
            internal_read_buffer_size: CBytes(32 * 1024),
            socket_write_buffer_size: Some(CBytes(32 * 1024)),
            socket_read_buffer_size: Some(CBytes(32 * 1024)),
            custom_headers: [
                (
                    http::header::USER_AGENT.to_string(),
                    vec!["Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/89.0.4389.128 Safari/537.36".into()],
                ),
                (
                    http::header::ACCEPT.to_string(),
                    vec!["text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9".into()],
                ),
                (
                    http::header::ACCEPT_ENCODING.to_string(),
                    vec!["gzip, deflate".into()],
                )
            ]
                .iter()
                .cloned()
                .collect(),
            max_response_size: CBytes(1024 * 1024 * 2),
            max_redirect: 5,
        }
    }
}