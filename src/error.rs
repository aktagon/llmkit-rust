use thiserror::Error;

///
///
///
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    ///
    ///
    ///
    #[error("validation: {field} - {message}")]
    Validation {
        field: &'static str,
        message: String,
    },
    #[error("unsupported: {0}")]
    Unsupported(String),
    ///
    ///
    ///
    ///
    ///
    ///
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
    ///
    ///
    ///
    ///
    ///
    ///
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

































