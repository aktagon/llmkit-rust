// Code generated — DO NOT EDIT.


use super::caching::ResourceLifecycleDef;
use super::providers::ProviderName;

// Batch contract constants shared by every SDK (ADR-091).

/// Prefix + the request index is the id sent with each batch request.
pub const BATCH_REQUEST_ID_PREFIX: &str = "req-";
/// `finish_reason` of a batch slot whose request has no result line.
pub const BATCH_SLOT_MISSING: &str = "missing";
/// `finish_reason` of a failed batch slot when the provider gives no reason.
pub const BATCH_SLOT_ERROR: &str = "error";

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
            request_count_paths: &["request_counts.processing", "request_counts.succeeded", "request_counts.errored", "request_counts.canceled", "request_counts.expired"],
            lifecycle: Some(&ResourceLifecycleDef {
                create_endpoint: "/v1/messages/batches",
                response_id_path: "id",
                reference_field: "",
                polling_endpoint: "",
                polling_status_path: "processing_status",
                polling_done_value: "ended",
                polling_error_values: &[],
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
            lifecycle: Some(&ResourceLifecycleDef {
                create_endpoint: "/v1/batches",
                response_id_path: "id",
                reference_field: "",
                polling_endpoint: "",
                polling_status_path: "status",
                polling_done_value: "completed",
                polling_error_values: &["failed", "expired", "cancelled"],
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
