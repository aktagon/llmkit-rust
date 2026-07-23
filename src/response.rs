use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::paths::{extract_f64_path, extract_string_path, extract_u32_path, set_wire_path};
use crate::providers::generated::caching::cache_usage_paths;
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::response::{usage_cost_path, usage_cost_scale};
use crate::{response_text_path, usage_paths, Provider, ProviderName, Response, Usage};

///
///
///
///
///
///
///
///
///
///
///
///
pub fn decode_response(
    provider: ProviderName,
    chat_wire_shape: &str,
    body: &str,
) -> Result<Response, Error> {
    let raw: Value = serde_json::from_str(body)?;

    if chat_wire_shape == "ChatResponsesOpenAI" {
        return Ok(parse_responses_envelope(&raw));
    }

    let text = extract_string_path(&raw, response_text_path(provider));
    let (input_path, output_path) = usage_paths(provider);
    let (write_path, read_path) = cache_usage_paths(provider);
    let cfg = provider_config(provider);
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
            cost: extract_f64_path(&raw, usage_cost_path(provider)) * usage_cost_scale(provider),
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
///
///
///
pub fn encode_response(
    provider: ProviderName,
    chat_wire_shape: &str,
    response: &Response,
) -> Result<String, Error> {
    guard_one_way_fields(provider, response)?;
    if chat_wire_shape == "ChatResponsesOpenAI" {
        return Ok(serde_json::to_string(&encode_responses_envelope(response))?);
    }

    let mut raw = Value::Object(Map::new());
    set_wire_path(&mut raw, response_text_path(provider), json!(response.text));
    let (input_path, output_path) = usage_paths(provider);
    set_wire_path(&mut raw, input_path, json!(response.usage.input));
    set_wire_path(&mut raw, output_path, json!(response.usage.output));
    let (write_path, read_path) = cache_usage_paths(provider);
    set_wire_path(&mut raw, write_path, json!(response.usage.cache_write));
    set_wire_path(&mut raw, read_path, json!(response.usage.cache_read));
    let scale = usage_cost_scale(provider);
    if scale != 0.0 {
        let cost = json!(response.usage.cost / scale);
        set_wire_path(&mut raw, usage_cost_path(provider), cost);
    }
    let cfg = provider_config(provider);
    set_wire_path(
        &mut raw,
        cfg.reasoning_tokens_path,
        json!(response.usage.reasoning),
    );
    set_wire_path(
        &mut raw,
        cfg.finish_reason_path,
        json!(response.finish_reason),
    );
    set_wire_path(
        &mut raw,
        cfg.finish_message_path,
        json!(response.finish_message),
    );
    Ok(serde_json::to_string(&raw)?)
}

///
///
///
///
///
///
///
///
fn guard_one_way_fields(provider: ProviderName, response: &Response) -> Result<(), Error> {
    if provider == ProviderName::Vertex && !response.finish_reason.is_empty() {
        return Err(Error::Validation {
            field: "response.finish_reason",
            message: "Vertex carries no finish-reason field. Its path reads predictions[0].raiFilteredReason — a safety-filter explanation surfaced AS the finish reason. Extraction is a deliberate fusion, so the reverse leg cannot decide whether a given canonical finish_reason originated as a safety verdict, and writing an ordinary stop signal into that field would fabricate one.".to_string(),
        });
    }
    Ok(())
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
///
///
fn encode_responses_envelope(response: &Response) -> Value {
    let mut raw = Value::Object(Map::new());
    if !response.text.is_empty() {
        raw["output"] = json!([{
            "type": "message",
            "content": [{"type": "output_text", "text": response.text}],
        }]);
    }
    set_wire_path(&mut raw, "usage.input_tokens", json!(response.usage.input));
    set_wire_path(
        &mut raw,
        "usage.output_tokens",
        json!(response.usage.output),
    );
    set_wire_path(
        &mut raw,
        "usage.input_tokens_details.cached_tokens",
        json!(response.usage.cache_read),
    );
    set_wire_path(
        &mut raw,
        "usage.output_tokens_details.reasoning_tokens",
        json!(response.usage.reasoning),
    );
    set_wire_path(&mut raw, "status", json!(response.finish_reason));
    raw
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
pub(crate) fn extract_finish_signal(raw: &Value, provider: ProviderName) -> (String, String) {
    let cfg = provider_config(provider);
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
