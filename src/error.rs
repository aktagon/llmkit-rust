use thiserror::Error;

/// Library-wide error type. Marked `#[non_exhaustive]` so new variants
/// can be added in 1.0.x without a SemVer break — match arms in
/// downstream code MUST include `_ =>` to handle future additions.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// Returned when a request, option, or builder field fails
    /// pre-flight validation. `field` is `&'static str` for now;
    /// can be relaxed to `String` post-1.0 (additive, non-breaking).
    #[error("validation: {field} - {message}")]
    Validation {
        field: &'static str,
        message: String,
    },
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The URL is stripped before storage (`reqwest::Error::without_url`) —
    /// for `QueryParamKey` auth (e.g. Google), the request URL carries the
    /// API key as `?key=<secret>`, and `reqwest::Error`'s `Display` embeds
    /// the full URL. Every `?`-propagated transport error in `http.rs`
    /// goes through this `From` impl (below), not `#[from]`, precisely so
    /// the redaction can't be bypassed at a call site.
    #[error("http: {0}")]
    Http(reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{provider}: {message} ({status_code})")]
    Api {
        provider: String,
        status_code: u16,
        message: String,
    },
    #[error("middleware veto: {0}")]
    MiddlewareVeto(String),
    /// The provider's event stream carried bytes that are not UTF-8 in a
    /// complete SSE line (BUG-063). Framing happens on bytes and decoding per
    /// line, so a multibyte character split across two chunks is never
    /// reported here; only a corrupt line is.
    #[error("stream: {0}")]
    Stream(String),
    /// The blocking `wait`/`wait_batch` deadline backstop fired before the job
    /// reached a terminal state (ADR-062 OQ-1 / ADR-063 POLL-008). Reachable
    /// ONLY from the blocking `wait` path — a single `poll` is one round-trip
    /// and never times out. A provider-reported failure is NOT this variant
    /// (it surfaces as `Error::Unsupported("<noun> failed: <msg>")`); branch on
    /// this to persist the handle and poll it later, or raise the deadline.
    #[error("poll: deadline exceeded for {provider} job {id}; the job may still be running — poll the handle across requests, or raise the deadline")]
    PollTimeout { provider: String, id: String },
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e.without_url())
    }
}

impl From<crate::middleware::MiddlewareVeto> for Error {
    fn from(value: crate::middleware::MiddlewareVeto) -> Self {
        Error::MiddlewareVeto(value.to_string())
    }
}

































