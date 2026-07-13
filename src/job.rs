//! Job engine (ADR-062 / ADR-063) — the ONE shared poll runtime for llmkit's
//! async, poll-until-done capabilities. Slice 1 migrates batch + transcription
//! onto it; video lands in slice 2. Mirror of `go/job.go` (the frozen
//! reference); behavior is faithful to it.
//!
//! Four "poll"-family names, kept deliberately distinct (glossary):
//!   - `poll()`    — the PUBLIC handle method (`BatchHandleExt::poll` /
//!                   `TranscriptionHandleExt::poll`): exactly one provider
//!                   round-trip, normalized, NO loop (ADR-063 POLL-001).
//!   - `poll_job`  — the internal engine: the bounded loop over `poll_once`
//!                   that owns the deadline backstop and the monotonic
//!                   `Running -> (Succeeded | Failed)` state machine.
//!   - `poll_once` — one engine iteration (poll -> classify -> result-when-
//!                   Succeeded). `poll()` IS `poll_once` made public; `wait` IS
//!                   `poll_job` (a loop over `poll_once`).
//!   - `PollBody`  — the once-decoded provider poll response; confines the
//!                   untyped JSON leaf so no bare `serde_json::Value` crosses an
//!                   adapter signature (S04).
//!   - `poll` (seam) — the adapter method that performs the round-trip and
//!                   returns a `PollBody`.
//!
//! The engine is generic on the result type (`JobAdapter::Out`) so no `Value`
//! crosses the seam (CLAUDE.md concrete-types rule; ADR-062 H1 typed-waist fix).

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::Error;
use crate::paths::extract_string_path;

/// The lifecycle state of an async job. PUBLIC because it is what `poll`
/// returns (ADR-063 POLL-004). The lifecycle is monotonic —
/// `Running -> (Succeeded | Failed)` — because `poll_job` returns on the FIRST
/// terminal classification and stores no state that could regress. There is
/// deliberately NO `Unknown`/zero member (ADR-063 §"Implementation refinements"
/// 2): `JobStatus` is always constructed with an explicit state, so a dead
/// public state would be misleading and asymmetric across the four SDKs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    /// Non-terminal: the job is submitted or in progress; keep polling. A
    /// reconstituted handle (ADR-014 cross-process resume) re-enters here.
    Running,
    /// Terminal success; the result is available.
    Succeeded,
    /// Terminal failure; see `JobStatus::cause`.
    Failed,
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            JobState::Running => "running",
            JobState::Succeeded => "succeeded",
            JobState::Failed => "failed",
        };
        f.write_str(s)
    }
}

/// The normalized failure detail carried by a `Failed` status. ONE terminal,
/// not a taxonomy (ADR-062 §1): the raw provider status, an optional provider
/// error message, and a `timed_out` flag. A consumer that needs the
/// expired-vs-cancelled distinction reads `status`; a typed cause enum is a
/// non-breaking follow-up (slice 2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobFailure {
    /// The raw provider status string that classified as failure (OpenAI batch
    /// "failed"/"expired"/"cancelled"; AssemblyAI "error").
    pub status: String,
    /// The provider error message when the provider reports one (AssemblyAI's
    /// top-level "error"); empty otherwise.
    pub message: String,
    /// True iff this failure is the engine's deadline backstop, not a
    /// provider-reported terminal. The blocking `wait` path surfaces the
    /// backstop as `Error::PollTimeout` instead, so `poll` never sets this true.
    pub timed_out: bool,
}

/// The normalized result of a single `poll` (ADR-063 POLL-001): the state plus
/// the result XOR the failure cause — never a raw provider payload. `result` is
/// `Some` iff `state == Succeeded`; `cause` is `Some` iff `state == Failed`.
#[derive(Clone, Debug)]
pub struct JobStatus<T> {
    /// The job's lifecycle state at this poll.
    pub state: JobState,
    /// The normalized capability response, set iff `state == Succeeded` (the
    /// second network hop, if any, has already been performed).
    pub result: Option<T>,
    /// The normalized failure detail, set iff `state == Failed`.
    pub cause: Option<JobFailure>,
    /// The provider's raw status string, for logging or a consumer that wants
    /// to branch below the normalized state.
    pub raw_status: String,
}

/// The config half of the engine seam: the classification facts (status path +
/// done / error value sets + the error-message path) and the poll cadence. Each
/// capability assembles it from its own generated facts.
#[derive(Clone, Debug)]
pub(crate) struct LifecycleConfig {
    /// Labels the capability in the failure error ("transcription", "batch") so
    /// a `Failed` terminal reads "<noun> failed: <message>" (S02).
    pub noun: &'static str,
    /// The provider name label, carried into `Error::PollTimeout` on the
    /// backstop path.
    pub provider: String,
    /// The job identifier, carried into `Error::PollTimeout` on the backstop.
    pub id: String,
    /// The dotted path to the status string in the poll body.
    pub status_path: String,
    /// The status strings marking terminal success (precedence over `error_values`).
    pub done_values: Vec<String>,
    /// The status strings marking terminal failure. An empty set means "no
    /// failure terminal" (Anthropic batch) — additive and backward-safe.
    pub error_values: Vec<String>,
    /// The dotted path to a provider error message, surfaced in
    /// `JobFailure::message`. Empty = no message extraction.
    pub error_message_path: String,
    /// The cadence between polls.
    pub poll_interval: Duration,
    /// The overall wall-clock backstop for the `poll_job` LOOP (NOT a
    /// per-request HTTP timeout, S05). Zero = no backstop.
    pub poll_timeout: Duration,
}

/// The once-decoded provider poll response (S04). Confines the untyped JSON
/// leaf: classification reads a config path via `status`; result reads the
/// decoded tree via `value`. No adapter signature carries a bare `Value`.
pub(crate) struct PollBody {
    raw: Value,
}

impl PollBody {
    pub(crate) fn new(raw: Value) -> Self {
        Self { raw }
    }

    /// The string at the given dotted path, or "" if absent.
    pub(crate) fn status(&self, path: &str) -> String {
        extract_string_path(&self.raw, path)
    }

    /// The decoded tree, for the capability `result` tail.
    pub(crate) fn value(&self) -> &Value {
        &self.raw
    }
}

/// What `classify` returns: the state plus the failure detail when `Failed`.
/// Internal — the public boundary is `JobState`.
pub(crate) struct Classification {
    pub state: JobState,
    pub failure: Option<JobFailure>,
    pub raw_status: String,
}

/// The shared config-driven default classifier (ADR-062 §a). Precedence
/// done > error > running: a status in `done_values` -> Succeeded; in
/// `error_values` -> Failed (message extracted); in NEITHER set -> Running (poll
/// on, bounded by the backstop). An unmodeled/new terminal degrades to a
/// bounded timeout — never a false success and never a false failure of a live
/// job (ADR-062 §"Implementation refinements" 4).
pub(crate) fn classify_by_config(lc: &LifecycleConfig, body: &PollBody) -> Classification {
    let status = body.status(&lc.status_path);
    if lc.done_values.iter().any(|d| *d == status) {
        return Classification {
            state: JobState::Succeeded,
            failure: None,
            raw_status: status,
        };
    }
    if lc.error_values.iter().any(|e| *e == status) {
        let mut failure = JobFailure {
            status: status.clone(),
            ..JobFailure::default()
        };
        if !lc.error_message_path.is_empty() {
            failure.message = body.status(&lc.error_message_path);
        }
        return Classification {
            state: JobState::Failed,
            failure: Some(failure),
            raw_status: status,
        };
    }
    Classification {
        state: JobState::Running,
        failure: None,
        raw_status: status,
    }
}

/// The capability seams the engine cannot share (ADR-062 difference table).
/// `classify` has a config-backed default (`classify_by_config`); `result` is
/// the capability tail and MAY perform a second network hop (batch's
/// output_file_id -> GET /content), so it is `async` and the adapter closes
/// over the http client + provider config.
///
/// `config` is the single source of the `LifecycleConfig`: `poll_job` reads it
/// for cadence/deadline and `classify` reads it for the done/error sets.
#[allow(async_fn_in_trait)]
pub(crate) trait JobAdapter {
    type Out;
    fn config(&self) -> &LifecycleConfig;
    async fn poll(&self) -> Result<PollBody, Error>;
    fn classify(&self, body: &PollBody) -> Result<Classification, Error>;
    async fn result(&self, body: &PollBody) -> Result<Self::Out, Error>;
}

/// Runs a single engine iteration: poll -> classify -> (on success) the
/// capability result tail, including any second network hop. It is `poll`'s body
/// and `poll_job`'s per-iteration step — no loop, no deadline (ADR-063
/// POLL-001: exactly one round-trip).
pub(crate) async fn poll_once<A: JobAdapter>(adapter: &A) -> Result<JobStatus<A::Out>, Error> {
    let body = adapter.poll().await?;
    let classification = adapter.classify(&body)?;
    let mut status = JobStatus {
        state: classification.state,
        result: None,
        cause: None,
        raw_status: classification.raw_status,
    };
    match classification.state {
        JobState::Succeeded => status.result = Some(adapter.result(&body).await?),
        JobState::Failed => status.cause = classification.failure,
        JobState::Running => {}
    }
    Ok(status)
}

/// The shared engine (ADR-062 §b). Loops `poll_once` on the configured cadence
/// until the first terminal classification or the deadline backstop.
/// Monotonicity is a consequence of returning on the first terminal, not of any
/// stored state. Rust callers cancel by dropping the future (the caller-side
/// equivalent of Go's ctx).
pub(crate) async fn poll_job<A: JobAdapter>(adapter: &A) -> Result<A::Out, Error> {
    let lc = adapter.config();
    let interval = if lc.poll_interval.is_zero() {
        Duration::from_secs(2)
    } else {
        lc.poll_interval
    };
    let deadline = if lc.poll_timeout.is_zero() {
        None
    } else {
        Some(Instant::now() + lc.poll_timeout)
    };
    loop {
        let status = poll_once(adapter).await?;
        match status.state {
            JobState::Succeeded => {
                return Ok(status
                    .result
                    .expect("Succeeded status carries a result by construction"));
            }
            JobState::Failed => {
                let failure = status
                    .cause
                    .expect("Failed status carries a cause by construction");
                return Err(job_failed_error(lc.noun, &failure));
            }
            JobState::Running => {}
        }
        // Still Running: fire the deadline backstop, then sleep. The backstop
        // surfaces as the typed `Error::PollTimeout` (POLL-008) — a provider
        // failure above is NOT this error.
        if let Some(deadline) = deadline {
            if Instant::now() > deadline {
                return Err(Error::PollTimeout {
                    provider: lc.provider.clone(),
                    id: lc.id.clone(),
                });
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Builds the error `poll_job` returns on a provider-reported terminal failure.
/// Its message preserves each capability's surface via `LifecycleConfig::noun` —
/// transcription's "transcription failed: <msg>" (S02). The deadline backstop is
/// NOT routed here (it returns `Error::PollTimeout`), so `timed_out` is never
/// formatted by this path.
fn job_failed_error(noun: &str, failure: &JobFailure) -> Error {
    let detail = if !failure.message.is_empty() {
        failure.message.as_str()
    } else {
        failure.status.as_str()
    };
    if detail.is_empty() {
        Error::Unsupported(format!("{noun} failed"))
    } else {
        Error::Unsupported(format!("{noun} failed: {detail}"))
    }
}

/// Filters out empty strings so a provider that leaves a status value unset
/// (e.g. no error status) contributes an empty set rather than a value that
/// would match a missing/empty poll status. Mirror of go `nonEmptyValues`.
pub(crate) fn non_empty_values<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    values
        .into_iter()
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_state_display() {
        assert_eq!(JobState::Running.to_string(), "running");
        assert_eq!(JobState::Succeeded.to_string(), "succeeded");
        assert_eq!(JobState::Failed.to_string(), "failed");
    }

    #[test]
    fn classify_precedence_done_over_error() {
        let lc = LifecycleConfig {
            noun: "batch",
            provider: "OpenAI".into(),
            id: "batch_1".into(),
            status_path: "status".into(),
            done_values: vec!["completed".into()],
            error_values: vec!["failed".into(), "expired".into()],
            error_message_path: String::new(),
            poll_interval: Duration::from_millis(1),
            poll_timeout: Duration::from_secs(1),
        };

        let done = classify_by_config(&lc, &PollBody::new(serde_json::json!({"status": "completed"})));
        assert_eq!(done.state, JobState::Succeeded);

        let failed = classify_by_config(&lc, &PollBody::new(serde_json::json!({"status": "expired"})));
        assert_eq!(failed.state, JobState::Failed);
        assert_eq!(failed.failure.unwrap().status, "expired");

        // An unmodeled status stays Running (safe degradation), never a false terminal.
        let unknown = classify_by_config(&lc, &PollBody::new(serde_json::json!({"status": "validating"})));
        assert_eq!(unknown.state, JobState::Running);
    }

    #[test]
    fn classify_extracts_error_message() {
        let lc = LifecycleConfig {
            noun: "transcription",
            provider: "Assemblyai".into(),
            id: "t-1".into(),
            status_path: "status".into(),
            done_values: vec!["completed".into()],
            error_values: vec!["error".into()],
            error_message_path: "error".into(),
            poll_interval: Duration::from_millis(1),
            poll_timeout: Duration::from_secs(1),
        };
        let body = PollBody::new(serde_json::json!({"status": "error", "error": "Download error"}));
        let c = classify_by_config(&lc, &body);
        assert_eq!(c.state, JobState::Failed);
        let f = c.failure.unwrap();
        assert_eq!(f.message, "Download error");
        assert!(!f.timed_out);
    }
}
