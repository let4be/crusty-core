#[allow(unused_imports)]
use crate::prelude::*;

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    future::Future,
    net::SocketAddr,
    net::ToSocketAddrs
};

use hyper::client::connect::dns::Name;
use hyper::service::Service;
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::TokioAsyncResolver;

#[derive(Debug, Clone)]
pub struct AsyncHyperResolver(TokioAsyncResolver);

impl AsyncHyperResolver {
    pub fn new(config: ResolverConfig, options: ResolverOpts) -> Result<Self, io::Error> {
        let resolver = TokioAsyncResolver::tokio(config, options)?;
        Ok(Self(resolver))
    }

    pub fn new_from_system_conf() -> Result<Self, io::Error> {
        let resolver = TokioAsyncResolver::tokio_from_system_conf()?;
        Ok(Self(resolver))
    }
}

impl Service<Name> for AsyncHyperResolver {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let resolver = self.0.clone();
        Box::pin((|| async move {
            Ok(resolver
                .lookup_ip(name.as_str())
                .await?
                .iter()
                .map(|addr| (addr, 0_u16).to_socket_addrs())
                .try_fold(Vec::new(), |mut acc, s_addr| {
                    acc.extend(s_addr?);
                    Ok::<_, io::Error>(acc)
                })?
                .into_iter())
        })())
    }
}