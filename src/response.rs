use serde_json::Value;

use crate::error::Error;
use crate::paths::{extract_f64_path, extract_string_path, extract_u32_path};
use crate::providers::generated::caching::cache_usage_paths;
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::response::{usage_cost_path, usage_cost_scale};
use crate::{response_text_path, usage_paths, Provider, Response, Usage};

pub fn parse_response(provider: &Provider, body: &str) -> Result<Response, Error> {
    let chat_wire_shape = provider_config(provider.name).chat_wire_shape;
    parse_response_shaped(provider, chat_wire_shape, body)
}

///
///
///
///
pub(crate) fn parse_response_shaped(
    provider: &Provider,
    chat_wire_shape: &str,
    body: &str,
) -> Result<Response, Error> {
    let raw: Value = serde_json::from_str(body)?;

    if chat_wire_shape == "ChatResponsesOpenAI" {
        return Ok(parse_responses_envelope(&raw));
    }

    let text = extract_string_path(&raw, response_text_path(provider.name));
    let (input_path, output_path) = usage_paths(provider.name);
    let (write_path, read_path) = cache_usage_paths(provider.name);
    let cfg = provider_config(provider.name);
    let reasoning = if cfg.reasoning_tokens_path.is_empty() {
        0
    } else {
        extract_u32_path(&raw, cfg.reasoning_tokens_path)
    };
    let (finish_reason, finish_message) = extract_finish_signal(&raw, provider);

    Ok(Response {
        text,
        usage: Usage {
            input: extract_u32_path(&raw, input_path),
            output: extract_u32_path(&raw, output_path),
            cache_write: extract_u32_path(&raw, write_path),
            cache_read: extract_u32_path(&raw, read_path),
            reasoning,
            cost: extract_f64_path(&raw, usage_cost_path(provider.name))
                * usage_cost_scale(provider.name),
        },
        finish_reason,
        finish_message,
        raw: None,
    })
}

///
///
///
///
///
///
///
fn parse_responses_envelope(raw: &Value) -> Response {
    Response {
        text: extract_responses_text(raw),
        usage: Usage {
            input: extract_u32_path(raw, "usage.input_tokens"),
            output: extract_u32_path(raw, "usage.output_tokens"),
            cache_write: 0,
            cache_read: extract_u32_path(raw, "usage.input_tokens_details.cached_tokens"),
            reasoning: extract_u32_path(raw, "usage.output_tokens_details.reasoning_tokens"),
            cost: 0.0,
        },
        finish_reason: extract_string_path(raw, "status"),
        finish_message: String::new(),
        raw: None,
    }
}

///
///
///
fn extract_responses_text(raw: &Value) -> String {
    let Some(output) = raw.get("output").and_then(Value::as_array) else {
        return String::new();
    };
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("output_text") {
                continue;
            }
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                return text.to_string();
            }
        }
    }
    String::new()
}

///
///
///
///
pub(crate) fn extract_finish_signal(raw: &Value, provider: &Provider) -> (String, String) {
    let cfg = provider_config(provider.name);
    let reason = if cfg.finish_reason_path.is_empty() {
        String::new()
    } else {
        extract_string_path(raw, cfg.finish_reason_path)
    };
    let message = if cfg.finish_message_path.is_empty() {
        String::new()
    } else {
        extract_string_path(raw, cfg.finish_message_path)
    };
    (reason, message)
}

pub fn parse_api_error(provider: &Provider, status_code: u16, body: &str) -> Error {
    let config = provider_config(provider.name);
    let parsed: Result<Value, _> = serde_json::from_str(body);
    let message = parsed
        .ok()
        .and_then(|raw| {
            if config.error_message_path.is_empty() {
                None
            } else {
                Some(extract_string_path(&raw, config.error_message_path))
            }
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| body.to_string());

    Error::Api {
        provider: config.slug.to_string(),
        status_code,
        message,
    }
}
