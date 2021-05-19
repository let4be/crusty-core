#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

pub enum LoadFilterAction {
    Skip,
    Term,
}

pub trait LoadFilter<JS: rt::JobStateValues, TS: rt::TaskStateValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &self,
        ctx: &rt::StdJobContext<JS, TS>,
        task: &rt::Task,
        status: &rt::Status,
    ) -> LoadFilterAction;
}
