use flume::bounded;

#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::{config, types::*};

#[derive(Clone)]
pub struct ParserProcessor {
	pool: Arc<rayon::ThreadPool>,
}

impl ParserProcessor {
	pub fn new(concurrency_profile: config::ConcurrencyProfile, stack_size_bytes: usize) -> ParserProcessor {
		let pool = rayon::ThreadPoolBuilder::new()
			.num_threads(concurrency_profile.parser_concurrency)
			.stack_size(stack_size_bytes)
			.build()
			.unwrap();

		Self { pool: Arc::new(pool) }
	}

	pub async fn spawn(&self, task: ParserTask) -> ParserResponse {
		let (tx, rx) = bounded::<ParserResponse>(1);
		self.pool.spawn(move || {
			let wait_time = task.time.elapsed();
			let t = Instant::now();

			let res = panic::catch_unwind(panic::AssertUnwindSafe(|| (task.payload)())).unwrap_or_else(|err| {
				Err(Error::FollowPanic { source: anyhow!("panic in parser processor {:?}", err) })
			});

			let work_time = t.elapsed();

			let _ = tx.send(ParserResponse { payload: res, wait_duration: wait_time, work_duration: work_time });
		});
		rx.recv_async().await.unwrap()
	}
}
