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
}

impl From<crate::middleware::MiddlewareVeto> for Error {
    fn from(value: crate::middleware::MiddlewareVeto) -> Self {
        Error::MiddlewareVeto(value.to_string())
    }
}
