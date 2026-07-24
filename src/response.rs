use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::paths::{extract_f64_path, extract_string_path, extract_u32_path, set_wire_path};
use crate::providers::generated::caching::cache_usage_paths;
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::response::{usage_cost_path, usage_cost_scale};
use crate::{response_text_path, usage_paths, Provider, ProviderName, Response, Usage};

/// Extracts text + usage from a provider response body into the canonical
/// [`Response`]. `chat_wire_shape` is the EFFECTIVE wire shape for this request
/// (after `.protocol(...)` resolution, ADR-055): only `ChatResponsesOpenAI`
/// diverges (the `output[]` envelope); every other value uses the provider's
/// declared response paths.
///
/// Keyless, IO-free and pure (ADR-076 SYM-002): no `Client`, no credential, no
/// network, no clock — the subject is the generated [`ProviderName`], not the
/// credential-carrying `Provider`. The wire shape is required, not derived —
/// one provider can serve two chat protocols, and inferring it silently
/// mis-parses (SYM-003). This is the same function the chat send path calls
/// (SYM-004).
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

/// [`decode_response`]'s inverse: renders a canonical [`Response`] back onto the
/// wire for `provider` + `chat_wire_shape`. Every write location comes from the
/// same generated path accessors `decode_response` reads — there is no second
/// table and no path literal here (ADR-076 SYM-005).
///
/// Keyless, IO-free and pure, like its mirror. The result is NOT byte-identical
/// to the body a provider would send: a provider body carries fields the
/// canonical `Response` does not model (ADR-014's `raw` exists for exactly
/// that). The contract is the canonical fixed point,
/// `decode(encode(decode(b))) == decode(b)` (SYM-006).
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

/// Refuses to encode a canonical field whose mapping is `OneWay` for this
/// provider — the result set of CQ-WMAP-011 (ADR-076 SYM-007). Only one member
/// is in Phase 2's scope; the other, `exceptGoogleToolCallID`, covers tool
/// calls, which are out (SYM-008).
///
/// An empty value is not an error: there is nothing to write, so the common
/// path stays usable and only the lying path fails. `field` and `message` carry
/// the mapping's `canonicalPath` and `invertibilityNote` verbatim.
fn guard_one_way_fields(provider: ProviderName, response: &Response) -> Result<(), Error> {
    if provider == ProviderName::Vertex && !response.finish_reason.is_empty() {
        return Err(Error::Validation {
            field: "response.finish_reason",
            message: "Vertex carries no finish-reason field. Its path reads predictions[0].raiFilteredReason — a safety-filter explanation surfaced AS the finish reason. Extraction is a deliberate fusion, so the reverse leg cannot decide whether a given canonical finish_reason originated as a safety verdict, and writing an ordinary stop signal into that field would fabricate one.".to_string(),
        });
    }
    Ok(())
}

/// Extracts text + usage from OpenAI's Responses reply (ADR-055). Unlike Chat
/// Completions (choices[].message.content), the reply is an `output[]` array
/// whose message item carries `content[]` blocks of type "output_text"; usage
/// is input_tokens/output_tokens with cached + reasoning sub-details.
/// Live-anchored 2026-07-02. Hand-coded per wire shape, symmetric with the
/// request-side `input` envelope (ADR-028: behavior held by tests, not by
/// declared response paths).
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

/// Mirror of `parse_responses_envelope`: rebuilds OpenAI's Responses reply
/// (ADR-055) — an `output[]` array whose message item carries `content[]`
/// blocks of type "output_text", with input_tokens/output_tokens usage and
/// cached + reasoning sub-details. Hand-coded per wire shape on both legs,
/// symmetric with the reader, for the same reason the reader is.
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

/// Walks the Responses `output[]` array for the first message item and returns
/// its first `output_text` block. Iterating (rather than a fixed
/// output[0].content[0] path) tolerates a leading reasoning item.
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

/// Pull the provider stop signal + free-text explanation from a response
///
/// empty strings when the provider declares no path or the value is not
/// present in this response.
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
