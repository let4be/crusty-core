#[allow(unused_imports)]
use crate::_prelude::*;
use crate::{config, types::*};

#[derive(Clone)]
pub struct ParserProcessor {
	concurrency:      usize,
	pin:              usize,
	stack_size_bytes: usize,
	rx:               Receiver<ParserTask>,
}

impl ParserProcessor {
	pub fn spawn(concurrency_profile: config::ConcurrencyProfile, stack_size_bytes: usize) -> Sender<ParserTask> {
		let (tx, rx) = bounded_ch::<ParserTask>(concurrency_profile.transit_buffer_size());

		let s = Self {
			concurrency: concurrency_profile.parser_concurrency,
			pin: concurrency_profile.parser_pin,
			stack_size_bytes,
			rx,
		};
		let _ = tokio::spawn(s.go());
		tx
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

	pub async fn go(mut self) -> Result<()> {
		let mut core_ids = core_affinity::get_core_ids().unwrap().into_iter();
		let handles: Vec<Result<std::thread::JoinHandle<()>>> = (0..self.concurrency)
			.into_iter()
			.map(|n| {
				let p = self.clone();
				let thread_builder = std::thread::Builder::new()
					.name(format!("parser processor {}", n))
					.stack_size(self.stack_size_bytes);

				let id = if self.pin > 0 {
					self.pin -= 1;
					core_ids.next()
				} else {
					None
				};
				let h = thread_builder
					.spawn(move || {
						if let Some(id) = id {
							core_affinity::set_for_current(id);
						}
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
