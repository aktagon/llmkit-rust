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
#
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

#
mod tests {
    use super::*;
    use crate::error::Error;

    //
    //
    #
    fn set_event_error_classifies_structurally() {
        let cases: Vec<(Error, &str)> = vec![
            (
                Error::Api {
                    provider: "openai".to_string(),
                    status_code: 429,
                    message: "rate limited".to_string(),
                },
                "api_error",
            ),
            (
                Error::Validation {
                    field: "model",
                    message: "model is required".to_string(),
                },
                "validation_error",
            ),
            (
                Error::Unsupported("cache create: empty resource ID".to_string()),
                "error",
            ),
            (
                Error::MiddlewareVeto("blocked by audit hook".to_string()),
                "error",
            ),
        ];
        for (err, expected_kind) in cases {
            let mut ev = Event::default();
            set_event_error(&mut ev, &err);
            assert_eq!(ev.err.as_deref(), Some(err.to_string().as_str()));
            assert_eq!(ev.err_type, expected_kind);
        }
    }
}
