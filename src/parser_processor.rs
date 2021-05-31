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

	pub fn install(&self, task: ParserTask) -> ParserResponse {
		self.pool.install(|| {
			let wait_time = task.time.elapsed();
			let t = Instant::now();
			let res = (task.payload)();
			let work_time = t.elapsed();

			ParserResponse { payload: res, wait_duration: wait_time, work_duration: work_time }
		})
	}
}
