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
    /// The blocking `wait`/`wait_batch`/`wait_video` deadline backstop fired
    /// before the job reached a terminal state (ADR-062 OQ-1 / ADR-063
    /// POLL-008). Reachable ONLY from the blocking `wait` path — a single
    /// `poll` is one round-trip and never times out. A provider-reported
    /// failure is NOT this variant (it is [`Error::JobFailed`]); branch on
    /// this to persist the handle and poll it later, or raise the deadline.
    #[error("poll: deadline exceeded for {provider} job {id}; the job may still be running — poll the handle across requests, or raise the deadline")]
    PollTimeout { provider: String, id: String },
    /// The provider ended an async job with a failure (BUG-103). Returned by
    /// the blocking `wait` paths of video generation, transcription and batch.
    /// The job is over: polling it again returns the same failure.
    ///
    /// - `job` names what ran: `"video generation"`, `"transcription"` or
    ///   `"batch"`.
    /// - `status` is the provider's raw terminal status, such as `"failed"`,
    ///   `"expired"`, `"FAILED"` or PixVerse's `"8"`. A Veo operation that
    ///   ends with an `error` object has the status `"error"`.
    /// - `message` is the provider's error message, empty when it sends none.
    ///
    /// A failed status check is NOT this variant: it is [`Error::Api`] with
    /// the HTTP status code, and the job may still be running.
    #[error("{}", job_failed_text(job, status, message))]
    JobFailed {
        job: &'static str,
        provider: String,
        id: String,
        status: String,
        message: String,
    },
    /// The provider's reply parsed but lacks a field llmkit needs to build
    /// the result (BUG-103), such as a finished video operation that carries
    /// no video. Unlike [`Error::JobFailed`], the provider reported success.
    #[error("decoding: {0}")]
    Decoding(String),
}

/// Renders `Error::JobFailed` as `"<job> failed: <detail>"`, where the detail
/// is the provider message, else the raw status. This keeps the text the
/// earlier `Error::Unsupported` failures carried.
fn job_failed_text(job: &str, status: &str, message: &str) -> String {
    let detail = if message.is_empty() { status } else { message };
    if detail.is_empty() {
        format!("{job} failed")
    } else {
        format!("{job} failed: {detail}")
    }
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





































