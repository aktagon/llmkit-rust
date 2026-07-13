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
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
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
    /// The blocking `wait`/`wait_batch` deadline backstop fired before the job
    /// reached a terminal state (ADR-062 OQ-1 / ADR-063 POLL-008). Reachable
    /// ONLY from the blocking `wait` path — a single `poll` is one round-trip
    /// and never times out. A provider-reported failure is NOT this variant
    /// (it surfaces as `Error::Unsupported("<noun> failed: <msg>")`); branch on
    /// this to persist the handle and poll it later, or raise the deadline.
    #[error("poll: deadline exceeded for {provider} job {id}; the job may still be running — poll the handle across requests, or raise the deadline")]
    PollTimeout { provider: String, id: String },
}

impl From<crate::middleware::MiddlewareVeto> for Error {
    fn from(value: crate::middleware::MiddlewareVeto) -> Self {
        Error::MiddlewareVeto(value.to_string())
    }
}
