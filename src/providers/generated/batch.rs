// Code generated — DO NOT EDIT.

use super::caching::ResourceLifecycleDef;
use super::providers::ProviderName;

// Batch contract constants shared by every SDK (ADR-091).

/// Prefix + the request index is the id sent with each batch request.
pub const BATCH_REQUEST_ID_PREFIX: &str = "req-";
/// `cause.code` of a batch slot whose request has no result line (ADR-099 CRF-016a).
pub const BATCH_SLOT_MISSING: JobFailureCode = JobFailureCode::Missing;
/// `cause.code` of a failed batch slot whose failure status has no entry in the
/// provider's table, or that gives none (ADR-099 CRF-016a).
pub const BATCH_SLOT_ERROR: JobFailureCode = JobFailureCode::Error;

/// Says why an async job, or one batch item, failed (ADR-099 CRF-016).
/// `JobFailure::code` holds one.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JobFailureCode {
    /// The job, or the batch that held the item, was cancelled before the work ran.
    Cancelled,
    /// The provider reported a failure. Also the code of a failure status that has no entry in the provider's table.
    Error,
    /// The job, or the batch that held the item, reached the end of its time window before the work ran.
    Expired,
    /// No result line came back for the batch item. It is a code, not a job state: a poll never returns it (ADR-063). JobFailure.Status is empty.
    Missing,
    /// The SDK's poll deadline passed while the provider still reported the job as running. The provider reported no failure, so JobFailure.Status is empty.
    Timeout,
}

impl JobFailureCode {
    /// The canonical string: the code's JobFailureCode label.
    pub fn as_str(self) -> &'static str {
        match self {
            JobFailureCode::Cancelled => "cancelled",
            JobFailureCode::Error => "error",
            JobFailureCode::Expired => "expired",
            JobFailureCode::Missing => "missing",
            JobFailureCode::Timeout => "timeout",
        }
    }
}

impl std::fmt::Display for JobFailureCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatchInputMode {
    InlineRequests,
    FileReferenceInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatchDef {
    pub input_mode: BatchInputMode,
    pub input_field: &'static str,
    pub file_purpose: &'static str,
    pub request_wrapper: &'static str,
    pub completion_window: &'static str,
    pub endpoint_path: &'static str,
    pub item_body_field: &'static str,
    pub result_body_path: &'static str,
    pub result_key_path: &'static str,
    pub result_status_path: &'static str,
    pub result_success_values: &'static [&'static str],
    pub result_reason_paths: &'static [&'static str],
    pub result_message_paths: &'static [&'static str],
    pub request_count_paths: &'static [&'static str],
    /// Failure status -> code; see failureCodeValue (ADR-099 CRF-016).
    pub result_failure_codes: &'static [(&'static str, JobFailureCode)],
    pub lifecycle: Option<&'static ResourceLifecycleDef>,
}

pub fn batch_config(provider: ProviderName) -> Option<&'static BatchDef> {
    match provider {
        ProviderName::Anthropic => Some(&BatchDef {
            input_mode: BatchInputMode::InlineRequests,
            input_field: "",
            file_purpose: "",
            request_wrapper: "requests",
            completion_window: "",
            endpoint_path: "",
            item_body_field: "params",
            result_body_path: "result.message",
            result_key_path: "custom_id",
            result_status_path: "result.type",
            result_success_values: &["succeeded"],
            result_reason_paths: &["result.type"],
            result_message_paths: &["result.error.error.message"],
            request_count_paths: &[
                "request_counts.processing",
                "request_counts.succeeded",
                "request_counts.errored",
                "request_counts.canceled",
                "request_counts.expired",
            ],
            result_failure_codes: &[
                ("canceled", JobFailureCode::Cancelled),
                ("errored", JobFailureCode::Error),
                ("expired", JobFailureCode::Expired),
            ],
            lifecycle: Some(&ResourceLifecycleDef {
                create_endpoint: "/v1/messages/batches",
                response_id_path: "id",
                reference_field: "",
                polling_endpoint: "",
                polling_status_path: "processing_status",
                polling_done_value: "ended",
                polling_error_values: &[],
                polling_failure_codes: &[],
                result_endpoint: "/v1/messages/batches/{id}/results",
                result_response_path: "",
                result_file_id_path: "",
                error_file_id_path: "",
                file_content_endpoint: "",
            }),
        }),
        ProviderName::Google => Some(&BatchDef {
            input_mode: BatchInputMode::InlineRequests,
            input_field: "",
            file_purpose: "",
            request_wrapper: "requests",
            completion_window: "",
            endpoint_path: "",
            item_body_field: "",
            result_body_path: "",
            result_key_path: "",
            result_status_path: "",
            result_success_values: &[],
            result_reason_paths: &[],
            result_message_paths: &[],
            request_count_paths: &[],
            result_failure_codes: &[],
            lifecycle: None,
        }),
        ProviderName::OpenAI => Some(&BatchDef {
            input_mode: BatchInputMode::FileReferenceInput,
            input_field: "input_file_id",
            file_purpose: "batch",
            request_wrapper: "",
            completion_window: "24h",
            endpoint_path: "/v1/chat/completions",
            item_body_field: "",
            result_body_path: "response.body",
            result_key_path: "custom_id",
            result_status_path: "response.status_code",
            result_success_values: &["200"],
            result_reason_paths: &["error.code", "response.body.error.code"],
            result_message_paths: &["error.message", "response.body.error.message"],
            request_count_paths: &["request_counts.total"],
            result_failure_codes: &[("batch_expired", JobFailureCode::Expired)],
            lifecycle: Some(&ResourceLifecycleDef {
                create_endpoint: "/v1/batches",
                response_id_path: "id",
                reference_field: "",
                polling_endpoint: "",
                polling_status_path: "status",
                polling_done_value: "completed",
                polling_error_values: &["failed", "expired", "cancelled"],
                polling_failure_codes: &[
                    ("cancelled", JobFailureCode::Cancelled),
                    ("expired", JobFailureCode::Expired),
                    ("failed", JobFailureCode::Error),
                ],
                result_endpoint: "",
                result_response_path: "",
                result_file_id_path: "output_file_id",
                error_file_id_path: "error_file_id",
                file_content_endpoint: "/v1/files/{id}/content",
            }),
        }),
        _ => None,
    }
}
