#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    types::*,
    config,
};

#[derive(Clone)]
pub struct ParserProcessor {
    concurrency: usize,
    stack_size_bytes: usize,
    rx: Receiver<ParserTask>
}

impl ParserProcessor {
    pub fn new(concurrency_profile: config::ConcurrencyProfile, stack_size_bytes: usize) -> (Self, Sender<ParserTask>) {
        let (tx, rx) = bounded_ch::<ParserTask>(concurrency_profile.transit_buffer_size() );

        (Self {concurrency: concurrency_profile.parser_concurrency, stack_size_bytes, rx}, tx)
    }

    fn process(&self, n: usize) -> PinnedTask {
        TracingTask::new(span!(Level::INFO, n=n), async move {
            while let Ok(task) = futures_lite::future::block_on(self.rx.recv()) {
                if task.res_tx.is_closed() {
                    continue;
                }

                let wait_time = task.time.elapsed();
                let t = Instant::now();
                let payload = task.payload;
                let res = payload();
                let work_time = t.elapsed();

                let _ = futures_lite::future::block_on(task.res_tx.send(ParserResponse {
                    res,
                    wait_time,
                    work_time
                }));
            }

            Ok(())
        }).instrument()
    }

    pub async fn go(self) -> Result<()>
    {
        let handles : Vec<Result<std::thread::JoinHandle<()>>> = (0..self.concurrency).into_iter().map(|n|{
            let p = self.clone();
            let thread_builder= std::thread::Builder::new().stack_size(self.stack_size_bytes);
            let h = thread_builder.spawn(move || {
                let _ = futures_lite::future::block_on(p.process(n));
            }).context("cannot spawn parser processor thread")?;
            Ok::<_, Error>(h)
        }).collect();
        for h in handles {
            let _ = h?.join();
        }
        Ok(())
    }
}