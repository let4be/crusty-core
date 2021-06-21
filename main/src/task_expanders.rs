#[allow(unused_imports)]
use crate::_prelude::*;
use crate::types as rt;

pub type Result = rt::ExtResult<()>;

pub trait Expander<JS: rt::JobStateValues, TS: rt::TaskStateValues, P> {
	fn name(&self) -> &'static str {
		"no name"
	}
	fn expand(&self, ctx: &mut rt::JobCtx<JS, TS>, task: &rt::Task, status: &rt::HttpStatus, document: &P) -> Result;
}
