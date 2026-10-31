use crate::middleware::MiddlewareFn;

/// The settings of a chat call that do not shape the request (ADR-098
/// RQC-002): send policy (caching), middleware and response handling (raw).
/// Every request-shaping option lives on [`crate::Request`].
#[derive(Clone, Default)]
pub(crate) struct SendOptions {
    pub(crate) caching: bool,
    pub(crate) cache_ttl: Option<u32>,
    pub(crate) middleware: Vec<MiddlewareFn>,
    /// Opt-in: populate `Response.raw` with the parsed provider response
    /// body (ADR-014). Plumbed by the typed builder's `.raw()` chain
    /// method on `*Text` and `*Agent`.
    pub(crate) raw: bool,
}
