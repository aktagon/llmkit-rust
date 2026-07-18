//!
//!
//!
//!
//!
//!

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

pub use crate::providers::generated::middleware::{Event, MiddlewareOp, MiddlewarePhase, Usage};

///
///
pub type MiddlewareFn =
    Arc<dyn Fn(&Event) -> Option<Box<dyn StdError + Send + Sync>> + Send + Sync>;

///
///
#[derive(Debug)]
pub struct MiddlewareVeto {
    pub cause: Box<dyn StdError + Send + Sync>,
}

impl fmt::Display for MiddlewareVeto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "middleware veto: {}", self.cause)
    }
}

impl StdError for MiddlewareVeto {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(self.cause.as_ref())
    }
}

///
///
///
///
pub fn set_event_error(ev: &mut Event, err: &crate::error::Error) {
    ev.err = Some(err.to_string());
    ev.err_type = match err {
        crate::error::Error::Api { .. } => "api_error",
        crate::error::Error::Validation { .. } => "validation_error",
        //
        //
        _ => "error",
    }
    .to_string();
}

///
///
pub fn fire_pre(mws: &[MiddlewareFn], base: &Event) -> Result<(), MiddlewareVeto> {
    if mws.is_empty() {
        return Ok(());
    }
    let mut ev = base.clone();
    ev.phase = MiddlewarePhase::Pre;
    for m in mws {
        if let Some(cause) = m(&ev) {
            return Err(MiddlewareVeto { cause });
        }
    }
    Ok(())
}

///
///
pub fn fire_post(mws: &[MiddlewareFn], base: &Event) {
    if mws.is_empty() {
        return;
    }
    let mut ev = base.clone();
    ev.phase = MiddlewarePhase::Post;
    for m in mws {
        let _ = m(&ev);
    }
}











































