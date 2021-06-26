use std::{
	net::ToSocketAddrs,
	task::{Context, Poll},
	vec::IntoIter,
};

use hyper::{client::connect::dns::Name, service::Service};
use ipnet::{Ipv4Net, Ipv6Net};
use trust_dns_resolver::{
	config::{ResolverConfig, ResolverOpts},
	TokioAsyncResolver,
};

use crate::_prelude::*;

pub trait Resolver: Send + Sync + Debug + 'static {
	fn resolve(&self, host: &str) -> PinnedFut<Result<IntoIter<SocketAddr>, io::Error>>;
}

#[derive(Clone, Debug)]
pub struct AsyncStaticResolver {
	addrs: Vec<SocketAddr>,
}

#[derive(Clone, Debug)]
pub struct AsyncTrustDnsResolver {
	resolver:         Arc<TokioAsyncResolver>,
	net_v4_blacklist: Arc<Vec<Ipv4Net>>,
	net_v6_blacklist: Arc<Vec<Ipv6Net>>,
}

impl AsyncTrustDnsResolver {
	pub fn new(config: ResolverConfig, options: ResolverOpts) -> Result<Self, io::Error> {
		let resolver = Arc::new(TokioAsyncResolver::tokio(config, options)?);

		Ok(Self { resolver, net_v4_blacklist: Arc::new(vec![]), net_v6_blacklist: Arc::new(vec![]) })
	}

	pub fn with_net_v4_blacklist(&mut self, blacklist: Vec<Ipv4Net>) {
		self.net_v4_blacklist = Arc::new(blacklist);
	}

	pub fn with_net_v6_blacklist(&mut self, blacklist: Vec<Ipv6Net>) {
		self.net_v6_blacklist = Arc::new(blacklist);
	}
}

impl AsyncStaticResolver {
	pub fn new(addrs: Vec<SocketAddr>) -> Self {
		Self { addrs }
	}
}

impl Resolver for AsyncTrustDnsResolver {
	fn resolve(&self, name: &str) -> PinnedFut<Result<IntoIter<SocketAddr>, io::Error>> {
		let resolver = self.clone();

		let name = name.to_string();
		Box::pin(async move {
			let r = resolver
				.resolver
				.lookup_ip(name.as_str())
				.await?
				.iter()
				.map(|addr| (addr, 0_u16).to_socket_addrs())
				.try_fold(Vec::new(), |mut acc, s_addr| {
					acc.extend(s_addr?);
					Ok::<_, io::Error>(acc)
				})?
				.into_iter();

			let mut err = None;
			let mut out_addrs: Vec<SocketAddr> = vec![];
			for addr in r.into_iter() {
				match addr {
					SocketAddr::V4(addr) => {
						for blacklisted_net in resolver.net_v4_blacklist.iter() {
							if blacklisted_net.contains(addr.ip()) {
								err = Some(io::Error::new(
									io::ErrorKind::Interrupted,
									format!("resolved IPv4 {} is blacklisted", addr.ip().to_string().as_str()),
								));
								continue
							}
						}
					}
					SocketAddr::V6(addr) => {
						for blacklisted_net in resolver.net_v6_blacklist.iter() {
							if blacklisted_net.contains(addr.ip()) {
								err = Some(io::Error::new(
									io::ErrorKind::Interrupted,
									format!("resolved IPv6 {} is blacklisted", addr.ip().to_string().as_str()),
								));
								continue
							}
						}
					}
				}
				out_addrs.push(addr);
			}

			if out_addrs.is_empty() {
				if let Some(err) = err {
					return Err(err)
				}
			}

			Ok(out_addrs.into_iter())
		})
	}
}

impl Resolver for AsyncStaticResolver {
	fn resolve(&self, _name: &str) -> PinnedFut<Result<IntoIter<SocketAddr>, io::Error>> {
		let resolver = self.clone();

		Box::pin(async move { Ok(resolver.addrs.into_iter()) })
	}
}

#[derive(Clone)]
pub(crate) struct Adaptor {
	resolver: Arc<Box<dyn Resolver>>,
}

impl Adaptor {
	pub fn new(resolver: Arc<Box<dyn Resolver>>) -> Self {
		Self { resolver }
	}
}

impl Service<Name> for Adaptor {
	type Error = io::Error;
	type Future = PinnedFut<Result<Self::Response, Self::Error>>;
	type Response = std::vec::IntoIter<SocketAddr>;

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	fn call(&mut self, name: Name) -> Self::Future {
		let adaptor = self.clone();
		Box::pin(async move { adaptor.resolver.resolve(name.as_str()).await })
	}
}

lazy_static! {
	pub static ref RESERVED_V4_SUBNETS: Vec<Ipv4Net> = {
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
		].iter().map(|net|net.parse().unwrap()).collect::<Vec<_>>()
	};

	pub static ref RESERVED_V6_SUBNETS: Vec<Ipv6Net> = {
		vec![
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
		].iter().map(|net|net.parse().unwrap()).collect::<Vec<_>>()
	};
}
