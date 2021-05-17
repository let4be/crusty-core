#[allow(unused_imports)]
use crate::prelude::*;
use crate::{
    types::*,
    config,
};

use std::thread::JoinHandle;

#[derive(Clone)]
pub struct ParserProcessor {
    concurrency: usize,
    stack_size_bytes: usize,
    rx: Receiver<ParserTask>
}

impl ParserProcessor {
    pub fn new(concurrency: usize, stack_size_bytes: usize, concurrency_profile: config::ConcurrencyProfile) -> (Self, Sender<ParserTask>) {
        let (tx, rx) = bounded_ch::<ParserTask>(concurrency_profile.transit_buffer_size() );

        (Self {concurrency, stack_size_bytes, rx}, tx)
    }

    fn process(&self, _n: usize) -> Result<()> {
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

        info!("Finished...");
        Ok(())
    }

    pub async fn go(self) -> Result<()>
    {
        for h in (0..self.concurrency).into_iter().map(|n| -> Result<JoinHandle<Result<()>>>{
            let h = std::thread::Builder::new().stack_size(self.stack_size_bytes).spawn({
                let p = self.clone();
                let n = n;
                move || {
                    p.process(n)
                }
            }).context("cannot spawn html parser thread")?;
            Ok(h)
        }) {
            let _ = h?.join();
        }
        Ok(())
    }
}