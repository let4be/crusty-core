#[allow(unused_imports)]
use crate::internal_prelude::*;

use std::{
    io,
    task::{Context, Poll},
    net::SocketAddr,
    net::ToSocketAddrs,
    vec::IntoIter,
};

use hyper::{
    client::connect::dns::Name,
    service::Service
};
use trust_dns_resolver::{
    config::{ResolverConfig, ResolverOpts},
    TokioAsyncResolver
};
use ipnet::{IpNet};

pub trait Resolver: Clone + Send + Sync + 'static {
    fn new_default() -> Result<Self, io::Error>;
    fn resolve(&self, host: &str) -> PinnedFut<Result<IntoIter<SocketAddr>, io::Error>>;
    fn with_net_blacklist(self, blacklist: Arc<Vec<IpNet>>) -> Self;
}

#[derive(Clone, Debug)]
pub struct AsyncHyperResolver {
    resolver: Arc<TokioAsyncResolver>,
    net_blacklist: Arc<Vec<IpNet>>,
}

impl AsyncHyperResolver {
    pub fn new(config: ResolverConfig, options: ResolverOpts) -> Result<Self, io::Error> {
        let resolver = Arc::new(TokioAsyncResolver::tokio(config, options)?);
        Ok(Self{resolver, net_blacklist: Arc::new(vec![])})
    }
}

impl Resolver for AsyncHyperResolver {
    fn new_default() -> Result<Self, io::Error> {
        let resolver = Arc::new(TokioAsyncResolver::tokio_from_system_conf()?);
        Ok(Self{resolver, net_blacklist: Arc::new(vec![])})
    }

    fn with_net_blacklist(mut self, blacklist: Arc<Vec<IpNet>>) -> Self {
        self.net_blacklist = blacklist;
        self
    }

    fn resolve(&self, name: &str) -> PinnedFut<Result<IntoIter<SocketAddr>, io::Error>> {
        let resolver = self.clone();
        let s_name  = String::from(name);

        Box::pin(async move {
            let r = resolver.resolver.lookup_ip(s_name.as_str())
                .await?
                .iter()
                .map(|addr| (addr, 0_u16).to_socket_addrs())
                .try_fold(Vec::new(), |mut acc, s_addr| {
                    acc.extend(s_addr?);
                    Ok::<_, io::Error>(acc)
                })?
                .into_iter();

            let mut out_addrs : Vec<SocketAddr> = vec![];
            for addr in r.into_iter() {
                for blacklisted_net in resolver.net_blacklist.iter() {
                    if blacklisted_net.contains(&addr.ip()) {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, format!("resolved IP {} is blacklisted", addr.ip().to_string().as_str())))
                    }
                }
                out_addrs.push(addr);
            }

            Ok(out_addrs.into_iter())
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Adaptor<R>{
    resolver: Arc<R>
}

impl<R: Resolver> Adaptor<R> {
    pub fn new(resolver: Arc<R>) -> Self {
        Self { resolver }
    }
}

impl<R: Resolver> Service<Name> for Adaptor<R> {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = io::Error;
    type Future = PinnedFut<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let adaptor = self.clone();
        Box::pin(async move {
            adaptor.resolver.resolve(name.as_str()).await
        })
    }
}

lazy_static! {
    pub static ref RESERVED_SUBNETS: Arc<Vec<IpNet>> = Arc::new({
        vec![
            // see https://www.iana.org/assignments/iana-ipv4-special-registry/iana-ipv4-special-registry.xhtml
            "0.0.0.0/8",
            "10.0.0.0/8",
            "100.64.0.0/10",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.0.0.0/24",
            "192.0.2.0/24",
            "192.88.99.0/24",
            "192.168.0.0/16",
            "198.18.0.0/15",
            "198.51.100.0/24",
            "203.0.113.0/24",
            "224.0.0.0/4",
            "233.252.0.0/24",
            "240.0.0.0/4",
            "255.255.255.255/32",

            // see https://www.iana.org/assignments/iana-ipv6-special-registry/iana-ipv6-special-registry.xhtml
            "::1/128",
            "::/128",
            "::ffff:0:0/96",
            "64:ff9b::/96",
            "64:ff9b:1::/48",
            "100::/64",
            "2001::/23",
            "2001::/32",
            "2001:1::1/128",
            "2001:1::2/128",
            "2001:2::/48",
            "2001:3::/32",
            "2001:4:112::/48",
            "2001:10::/28",
            "2001:20::/28",
            "2001:db8::/32",
            "2002::/16",
            "2620:4f:8000::/48",
            "fc00::/7",
            "fe80::/10",
        ].iter().map(|net|net.parse().unwrap()).collect()
    });
}