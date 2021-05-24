#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

pub enum LoadFilterAction {
    Skip,
    Term,
}

pub trait Filter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &self,
        ctx: &rt::JobContext<JS, TS>,
        task: &rt::Task,
        status: &rt::Status,
    ) -> LoadFilterAction;
}
