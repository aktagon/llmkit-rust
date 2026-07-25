//! Middleware runtime — pre-phase veto + post-phase observation.
//!
//! Mirrors go/middleware.go. The handwritten runtime is responsible for
//! firing middleware around each operation site; the generated
//! `Event` / `MiddlewareOp` / `MiddlewarePhase` types live alongside in
//! `providers::generated::middleware`.

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;

pub use crate::providers::generated::middleware::{Event, MiddlewareOp, MiddlewarePhase};

/// User-supplied middleware hook. Pre-phase non-`None` return vetoes the
/// operation; post-phase return values are discarded.
pub type MiddlewareFn =
    Arc<dyn Fn(&Event) -> Option<Box<dyn StdError + Send + Sync>> + Send + Sync>;

/// Wraps a pre-phase veto cause so callers can match against it via
/// `match err { llmkit::Error::MiddlewareVeto(_) => ... }`.
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

/// Erase a typed error onto a post-phase `Event`: `err` (the human string)
/// and `err_type` (the ADR-071 structural kind the OTLP builder reads
/// verbatim). Classification happens here — the one seam where the typed
/// `Error` still exists — never by re-parsing the `Display` string.
pub fn set_event_error(ev: &mut Event, err: &crate::error::Error) {
    ev.err = Some(err.to_string());
    ev.err_type = match err {
        crate::error::Error::Api { .. } => "api_error",
        crate::error::Error::Validation { .. } => "validation_error",
        // Transport, decoding, unsupported, veto, timeout: the stable
        // catch-all kind.
        _ => "error",
    }
    .to_string();
}

/// Run pre-phase middlewares in registration order. First non-`None`
/// return aborts and is wrapped as `MiddlewareVeto`.
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

/// Run post-phase middlewares in registration order. Return values are
/// discarded — post-phase is strictly observational.
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











































