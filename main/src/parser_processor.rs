use actix::{Handler, SyncArbiter};

use crate::{_prelude::*, config, types::*};

#[derive(Clone)]
pub struct ParserActor {}

impl Actor for ParserActor {
	type Context = SyncContext<Self>;
}

impl Handler<ParserTask> for ParserActor {
	type Result = ParserResponse;

	fn handle(&mut self, task: ParserTask, _ctx: &mut SyncContext<Self>) -> Self::Result {
		let wait_time = task.time.elapsed();
		let t = Instant::now();
		let res = (task.payload)();
		let work_time = t.elapsed();

		ParserResponse { payload: res, wait_duration: wait_time, work_duration: work_time }
	}
}

impl ParserActor {
	pub fn spawn(concurrency_profile: config::ConcurrencyProfile, _stack_size_bytes: usize) -> Addr<ParserActor> {
		SyncArbiter::start(concurrency_profile.parser_concurrency, || ParserActor {})
	}
}
