use std::task::{Context, Poll};

use hyper::{
	client::connect::{Connected, Connection},
	service::Service,
	Uri,
};
use pin_project::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[allow(unused_imports)]
use crate::internal_prelude::*;

#[derive(Clone, Debug)]
pub struct Stats {
	read:  Arc<Mutex<usize>>,
	write: Arc<Mutex<usize>>,
}

impl Stats {
	#[allow(clippy::mutex_atomic)]
	pub fn new() -> Self {
		Self { read: Arc::new(Mutex::new(0)), write: Arc::new(Mutex::new(0)) }
	}

	fn inc_read(&mut self, s: usize) {
		let mut read = self.read.lock().unwrap();
		*read += s;
	}

	pub fn read(&self) -> usize {
		*self.read.lock().unwrap()
	}

	fn inc_write(&mut self, s: usize) {
		let mut write = self.write.lock().unwrap();
		*write += s;
	}

	pub fn write(&self) -> usize {
		*self.write.lock().unwrap()
	}

	pub fn reset(&mut self) {
		let mut read = self.read.lock().unwrap();
		let mut write = self.write.lock().unwrap();
		*read = 0;
		*write = 0;
	}
}

#[pin_project]
#[derive(Debug)]
pub struct CountingStream<T> {
	#[pin]
	stream: T,
	#[pin]
	stats:  Stats,
}

impl<T> CountingStream<T> {
	fn new(stream: T, stats: Stats) -> Self {
		Self { stream, stats }
	}
}

impl<T: AsyncRead + AsyncWrite + Unpin> AsyncWrite for CountingStream<T> {
	fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize, io::Error>> {
		let mut s = self.project();

		match s.stream.poll_write(cx, buf) {
			Poll::Ready(Ok(n)) => {
				s.stats.inc_write(n);
				Poll::Ready(Ok(n))
			}
			Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
			Poll::Pending => Poll::Pending,
		}
	}

	fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
		self.project().stream.poll_flush(cx)
	}

	fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
		self.project().stream.poll_shutdown(cx)
	}
}

impl<T: AsyncWrite + AsyncRead + Unpin> AsyncRead for CountingStream<T> {
	fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
		let mut s = self.project();
		let len_before = buf.filled().len();

		match s.stream.poll_read(cx, buf) {
			Poll::Ready(Ok(())) => {
				s.stats.inc_read(buf.filled().len() - len_before);
				Poll::Ready(Ok(()))
			}
			Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
			Poll::Pending => Poll::Pending,
		}
	}
}

impl<T: AsyncRead + AsyncWrite + Connection + Unpin> Connection for CountingStream<T> {
	fn connected(&self) -> Connected {
		self.stream.connected()
	}
}

#[derive(Clone, Debug)]
pub struct CountingConnector<R> {
	c:         R,
	pub stats: Stats,
}

impl<R> CountingConnector<R> {
	pub fn new(c: R) -> Self {
		Self { c, stats: Stats::new() }
	}
}

impl<R> Unpin for CountingConnector<R> {}

impl<R> Service<Uri> for CountingConnector<R>
where
	R: Service<Uri>,
	R::Response: AsyncRead + AsyncWrite + Send + Unpin,
	R::Future: Send + 'static,
	R::Error: Into<BoxError>,
{
	type Error = BoxError;
	type Future = Connecting<R::Response>;
	type Response = CountingStream<R::Response>;

	fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		match self.c.poll_ready(cx) {
			Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
			Poll::Ready(Err(e)) => Poll::Ready(Err(e.into())),
			Poll::Pending => Poll::Pending,
		}
	}

	fn call(&mut self, dst: Uri) -> Self::Future {
		let connecting = self.c.call(dst);
		let stats = self.stats.clone();
		let fut = async move {
			let s = connecting.await.map_err(Into::into)?;
			Ok(CountingStream::new(s, stats))
		};

		Connecting(Box::pin(fut))
	}
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub struct Connecting<T>(PinnedFut<Result<CountingStream<T>, BoxError>>);

impl<T: AsyncRead + AsyncWrite + Unpin> Future for Connecting<T> {
	type Output = Result<CountingStream<T>, BoxError>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		Pin::new(&mut self.0).poll(cx)
	}
}
