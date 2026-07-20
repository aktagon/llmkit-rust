//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::time::{Duration, Instant};

use serde_json::Value;

use crate::error::Error;
use crate::paths::extract_string_path;

///
///
///
///
///
///
///
#
pub enum JobState {
    ///
    ///
    Running,
    ///
    Succeeded,
    ///
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

///
///
///
///
///
#
pub struct JobFailure {
    ///
    ///
    pub status: String,
    ///
    ///
    pub message: String,
    ///
    ///
    ///
    pub timed_out: bool,
}

///
///
///
#
pub struct JobStatus<T> {
    ///
    pub state: JobState,
    ///
    ///
    pub result: Option<T>,
    ///
    pub cause: Option<JobFailure>,
    ///
    ///
    pub raw_status: String,
}

///
///
///
#
pub(crate) struct LifecycleConfig {
    ///
    ///
    pub noun: &'static str,
    ///
    ///
    pub provider: String,
    ///
    pub id: String,
    ///
    pub status_path: String,
    ///
    pub done_values: Vec<String>,
    ///
    ///
    pub error_values: Vec<String>,
    ///
    ///
    pub error_message_path: String,
    ///
    pub poll_interval: Duration,
    ///
    ///
    pub poll_timeout: Duration,
}

///
///
///
pub(crate) struct PollBody {
    raw: Value,
}

impl PollBody {
    pub(crate) fn new(raw: Value) -> Self {
        Self { raw }
    }

    ///
    pub(crate) fn status(&self, path: &str) -> String {
        extract_string_path(&self.raw, path)
    }

    ///
    pub(crate) fn value(&self) -> &Value {
        &self.raw
    }
}

///
///
pub(crate) struct Classification {
    pub state: JobState,
    pub failure: Option<JobFailure>,
    pub raw_status: String,
}

///
///
///
///
///
///
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

///
///
///
///
///
///
///
///
#
pub(crate) trait JobAdapter {
    type Out;
    fn config(&self) -> &LifecycleConfig;
    async fn poll(&self) -> Result<PollBody, Error>;
    fn classify(&self, body: &PollBody) -> Result<Classification, Error>;
    async fn result(&self, body: &PollBody) -> Result<Self::Out, Error>;
}

///
///
///
///
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

///
///
///
///
///
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
        //
        //
        //
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

///
///
///
///
///
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

///
///
///
pub(crate) fn non_empty_values<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    values
        .into_iter()
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect()
}

#
mod tests {
    use super::*;

    #
    fn job_state_display() {
        assert_eq!(JobState::Running.to_string(), "running");
        assert_eq!(JobState::Succeeded.to_string(), "succeeded");
        assert_eq!(JobState::Failed.to_string(), "failed");
    }

    #
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

        //
        let unknown = classify_by_config(&lc, &PollBody::new(serde_json::json!({"status": "validating"})));
        assert_eq!(unknown.state, JobState::Running);
    }

    #
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
