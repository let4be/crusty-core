#[allow(unused_imports)]
use crate::prelude::*;

use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    future::Future,
    net::SocketAddr,
    net::ToSocketAddrs,
    sync::Arc
};

use hyper::client::connect::dns::Name;
use hyper::service::Service;
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::TokioAsyncResolver;
use std::vec::IntoIter;

pub type PinnedFut<T> = Pin<Box<dyn Future<Output=Result<T, io::Error>> + Send>>;
pub trait Resolver: Clone + Send + Sync + 'static {
    fn new_default() -> Result<Self, io::Error>;
    fn resolve(&self, host: &str) -> PinnedFut<IntoIter<SocketAddr>>;
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

    fn resolve(&self, name: &str) -> PinnedFut<IntoIter<SocketAddr>> {
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
pub(crate) struct AsyncHyperResolverAdaptor<R>{
    resolver: Arc<R>
}

impl<R: Resolver> AsyncHyperResolverAdaptor<R> {
    pub fn new(resolver: Arc<R>) -> AsyncHyperResolverAdaptor<R> {
        Self { resolver }
    }
}

impl<R: Resolver> Service<Name> for AsyncHyperResolverAdaptor<R> {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let resolver = self.resolver.clone();
        Box::pin(async move {
            Ok(resolver.resolve(name.as_str()).await?)
        })
    }
}