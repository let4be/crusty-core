#[allow(unused_imports)]
use crate::internal_prelude::*;
use crate::types as rt;

pub enum Action {
    Skip,
    Term,
}

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &self,
        ctx: &rt::JobCtx<JS, TS>,
        task: &rt::Task,
        status: &rt::HttpStatus,
        reader: Box<dyn io::Read + Sync + Send>
    ) -> Action;
}
