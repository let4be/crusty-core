#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{config, types::*};

#[derive(Clone)]
pub struct ParserProcessor {
	concurrency:      usize,
	stack_size_bytes: usize,
	rx:               Receiver<ParserTask>,
}

pub struct Handle {
	pub(crate) tx: Sender<ParserTask>,
	h:             tokio::task::JoinHandle<Result<()>>,
}

impl Handle {
	pub fn join(self) -> tokio::task::JoinHandle<Result<()>> {
		drop(self.tx);
		self.h
	}
}

impl ParserProcessor {
	pub fn spawn(concurrency_profile: config::ConcurrencyProfile, stack_size_bytes: usize) -> Handle {
		let (tx, rx) = bounded_ch::<ParserTask>(concurrency_profile.transit_buffer_size());

		let s = Self { concurrency: concurrency_profile.parser_concurrency, stack_size_bytes, rx };
		let h = tokio::spawn(s.go());
		Handle { tx, h }
	}

	fn process(&self, n: usize) -> PinnedTask {
		TracingTask::new(span!(n = n), async move {
			while let Ok(task) = self.rx.recv() {
				if task.res_tx.is_disconnected() {
					continue
				}

				let wait_time = task.time.elapsed();
				let t = Instant::now();
				let res = (task.payload)();
				let work_time = t.elapsed();

				let _ = task.res_tx.send(ParserResponse {
					payload:       res,
					wait_duration: wait_time,
					work_duration: work_time,
				});
			}

			Ok(())
		})
		.instrument()
	}

	pub async fn go(self) -> Result<()> {
		let handles: Vec<Result<std::thread::JoinHandle<()>>> = (0..self.concurrency)
			.into_iter()
			.map(|n| {
				let p = self.clone();
				let thread_builder = std::thread::Builder::new().stack_size(self.stack_size_bytes);
				let h = thread_builder
					.spawn(move || {
						let _ = futures_lite::future::block_on(p.process(n));
					})
					.context("cannot spawn parser processor thread")?;
				Ok::<_, Error>(h)
			})
			.collect();
		for h in handles {
			let _ = h?.join();
		}
		Ok(())
	}
}
