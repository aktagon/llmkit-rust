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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
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
#[allow(async_fn_in_trait)]
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



























































