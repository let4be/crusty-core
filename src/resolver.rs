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

pub trait Resolver: Clone + Send + Sync + 'static {
    fn new_default() -> Result<Self, io::Error>;
    fn resolve(&self, host: &str) -> PinnedFut<Result<IntoIter<SocketAddr>, io::Error>>;
}

#[derive(Clone, Debug)]
pub struct AsyncHyperResolver {
    resolver: Arc<TokioAsyncResolver>,
}

impl AsyncHyperResolver {
    pub fn new(config: ResolverConfig, options: ResolverOpts) -> Result<Self, io::Error> {
        let resolver = Arc::new(TokioAsyncResolver::tokio(config, options)?);
        Ok(Self{resolver})
    }
}

impl Resolver for AsyncHyperResolver {
    fn new_default() -> Result<Self, io::Error> {
        let resolver = Arc::new(TokioAsyncResolver::tokio_from_system_conf()?);
        Ok(Self{resolver})
    }

    fn resolve(&self, name: &str) -> PinnedFut<Result<IntoIter<SocketAddr>, io::Error>> {
        let resolver = Arc::clone(&self.resolver);
        let s_name  = String::from(name);

        Box::pin(async move {
            let r = resolver.lookup_ip(s_name.as_str())
                .await?
                .iter()
                .map(|addr| (addr, 0_u16).to_socket_addrs())
                .try_fold(Vec::new(), |mut acc, s_addr| {
                    acc.extend(s_addr?);
                    Ok::<_, io::Error>(acc)
                })?
                .into_iter();
            Ok(r)
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Adaptor<R>{
    resolver: Arc<R>
}

impl<R: Resolver> Adaptor<R> {
    pub fn new(resolver: Arc<R>) -> Adaptor<R> {
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
        let resolver = Arc::clone(&self.resolver);
        Box::pin(async move {
            resolver.resolve(name.as_str()).await
        })
    }
}