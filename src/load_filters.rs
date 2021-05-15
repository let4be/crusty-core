#[allow(unused_imports)]
use crate::prelude::*;
use crate::types as rt;

pub enum LoadFilterAction {
    Skip,
    Term,
}

pub trait LoadFilter<T: rt::JobContextValues> {
    fn name(&self) -> &'static str;
    fn accept(
        &self,
        ctx: &rt::StdJobContext<T>,
        task: &rt::Task,
        status: &rt::Status,
    ) -> LoadFilterAction;
}
