use crate::structs::{Part, Response};
use serde_json::Value;

use crate::error::Error;
use crate::options::SendOptions;
use crate::paths::{extract_string_path, opt_int_path};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::providers::generated::stream::{
    stream_config, wire_shape_stream_config, wire_shape_stream_finish_reason_path,
    wire_shape_stream_response_id_path, wire_shape_stream_response_model_path, StreamDef,
};
use crate::request::{request_path, send_headers, send_url};
use crate::response::{decode_finish_reason, opt_string};
use crate::structs::Request;
use crate::types::{Provider, Usage};

pub async fn prompt_stream<F>(
    provider: &Provider,
    request: &Request,
    protocol: &str,
    options: &SendOptions,
    mut callback: F,
) -> Result<Response, Error>
where
    F: FnMut(&str),
{
    // ADR-055: a non-default protocol (Responses) overrides the endpoint and
    // wire shape, and the wire shape selects the stream facts (HANDOFF-087).
    let base_config = provider_config(provider.name);
    let effective = crate::request::resolve_chat_protocol(base_config, protocol)?;
    let config = &effective;
    let (stream, locators) =
        stream_settings(provider, config).ok_or_else(|| Error::Validation {
            field: "provider".into(),
            message: format!("streaming not supported: {:?}", provider.name),
        })?;

    let crate::request::ChatRequest {
        request,
        mut body,
        headers: content,
    } = crate::request::encode_chat(provider, config, request)?;
    let url = build_stream_url(provider, config, stream, &request.model);
    let headers = send_headers(provider, config, &content);
    crate::caching::apply_caching(&mut body, provider, &request.model, options, config).await?;
    if !stream.param.is_empty() {
        if let Some(object) = body.as_object_mut() {
            object.insert(stream.param.to_string(), Value::Bool(true));
        }
    }
    // BUG-028: opt into a streamed usage frame where the provider requires it.
    if stream.usage_opt_in {
        if let Some(object) = body.as_object_mut() {
            object.insert(
                "stream_options".to_string(),
                serde_json::json!({ "include_usage": true }),
            );
        }
    }

    let client = crate::http::client_for(provider.timeout)?;
    let mut request_builder = crate::http::json_post(&client, &url, &body)?;
    for (name, value) in &headers {
        request_builder = request_builder.header(name, value);
    }

    let response = request_builder.send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await?;
        return Err(crate::response::parse_api_error(
            provider,
            status.as_u16(),
            &body,
        ));
    }

    let finish_loc = parse_stream_finish_path(locators.finish_reason);
    let id_loc = parse_stream_finish_path(locators.id);
    let model_loc = parse_stream_finish_path(locators.model);

    let mut usage = Usage::default();
    let mut full_text = String::new();
    let mut fields = StreamFields::default();
    let mut current_event = String::new();
    let mut framer = SseFramer::default();
    // reqwest's wasm32 client cannot read a body in chunks without its
    // `stream` feature. In the browser the whole body arrives first, so the
    // callback fires for every delta at once, after the response completes.
    #[cfg(not(target_arch = "wasm32"))]
    let mut response = response;
    #[cfg(target_arch = "wasm32")]
    let mut whole_body = Some(response.bytes().await?);

    loop {
        #[cfg(not(target_arch = "wasm32"))]
        let Some(chunk) = response.chunk().await?
        else {
            break;
        };
        #[cfg(target_arch = "wasm32")]
        let Some(chunk) = whole_body.take() else {
            break;
        };
        for line in framer.push(&chunk) {
            if let Some(event) = line.strip_prefix("event: ") {
                current_event = event.to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };

            // Data-level done sentinel (e.g., OpenAI [DONE]) is literal, not JSON.
            if !stream.done_signal.is_empty() && data == stream.done_signal {
                return Ok(fields.into_response(config.chat_wire_shape, full_text, usage));
            }

            let parsed: Option<Value> = serde_json::from_str(data).ok();

            // ADR-013: capture stream-time finish-reason BEFORE the event-level
            // done return, so a finish locator that names the done event still
            // sees its body. (Anthropic's is message_delta:delta.stop_reason.)
            if let Some(ref parsed_value) = parsed {
                let value = read_locator(finish_loc, parsed_value, &current_event);
                if !value.is_empty() && value != "FINISH_REASON_UNSPECIFIED" {
                    fields.finish_reason = Some(value);
                }
                let value = read_locator(id_loc, parsed_value, &current_event);
                if !value.is_empty() {
                    fields.id = Some(value);
                }
                let value = read_locator(model_loc, parsed_value, &current_event);
                if !value.is_empty() {
                    fields.model = Some(value);
                }
            }

            if stream.uses_event_types
                && !stream.done_event.is_empty()
                && current_event == stream.done_event
            {
                return Ok(fields.into_response(config.chat_wire_shape, full_text, usage));
            }

            let Some(parsed) = parsed else {
                current_event.clear();
                continue;
            };

            if stream.uses_event_types {
                if current_event == stream.content_event {
                    let text = extract_string_path(&parsed, stream.delta_text_path);
                    if !text.is_empty() {
                        full_text.push_str(&text);
                        callback(&text);
                    }
                }
                if current_event == stream.usage_event {
                    usage.output = opt_int_path(&parsed, stream.usage_output_path);
                    usage.input = opt_int_path(&parsed, stream.usage_input_path);
                }
            } else {
                let text = extract_string_path(&parsed, stream.delta_text_path);
                if !text.is_empty() {
                    full_text.push_str(&text);
                    callback(&text);
                }
                // Usage arrives in ONE late frame; every earlier frame carries
                // none. The gate is therefore "did this frame report it", not
                // "is the number big enough" — the old `> 0` test also threw
                // away a genuinely reported zero (ADR-081 AVAIL-001).
                if let Some(value) = opt_int_path(&parsed, stream.usage_input_path) {
                    usage.input = Some(value);
                }
                if let Some(value) = opt_int_path(&parsed, stream.usage_output_path) {
                    usage.output = Some(value);
                }
            }

            current_event.clear();
        }
    }

    Ok(fields.into_response(config.chat_wire_shape, full_text, usage))
}

/// The ADR-013 locators a stream reads its finish reason, response id and
/// model from (ADR-099 CRF-014). Empty disables a capture.
struct StreamLocators {
    finish_reason: &'static str,
    id: &'static str,
    model: &'static str,
}

/// The values the locators found; each `None` when no frame matched.
#[derive(Default)]
struct StreamFields {
    finish_reason: Option<String>,
    id: Option<String>,
    model: Option<String>,
}

impl StreamFields {
    /// The stream reads the same table as the one-shot body (ADR-099 CRF-008).
    /// It keeps no body, so a fused value (CRF-007) cannot be told apart and
    /// decodes to `Other`.
    fn into_response(self, chat_wire_shape: &str, text: String, usage: Usage) -> Response {
        Response {
            parts: if text.is_empty() {
                Vec::new()
            } else {
                vec![Part::Text(text)]
            },
            usage,
            finish_reason: decode_finish_reason(
                chat_wire_shape,
                self.finish_reason.as_deref(),
                None,
            ),
            provider_finish_reason: self.finish_reason,
            id: self.id,
            model: self.model,
            chat_wire_shape: opt_string(chat_wire_shape.to_owned()),
            ..Response::default()
        }
    }
}

/// The string at a split locator's path in a frame of its event, or "" when
/// the frame does not match or carries no value there.
fn read_locator((event, path): (&str, &str), parsed: &Value, current_event: &str) -> String {
    if path.is_empty() || (!event.is_empty() && event != current_event) {
        return String::new();
    }
    extract_string_path(parsed, path)
}

/// Frames an SSE byte stream into lines, then decodes each complete line.
///
/// Framing before decoding is the whole point (BUG-063). Provider chunk
/// boundaries are arbitrary, so a multibyte UTF-8 character regularly
/// straddles two chunks; decoding each chunk on its own turned both halves
/// into U+FFFD with no error. Bytes are buffered until a `\n` arrives, and
/// only a complete line is decoded, so a split character is whole by the time
/// it is read. A byte that is still not UTF-8 in a complete line is corrupt
/// and decodes to U+FFFD, the same as Go, TS, Python, Swift and Java do
/// (decision 2026-09-04: lossy per line in every SDK, never an error).
#[derive(Default)]
pub(crate) struct SseFramer {
    buffer: Vec<u8>,
}

impl SseFramer {
    /// Append `chunk` and return every line it completes, in order, with the
    /// trailing `\n` and any `\r` removed. Bytes after the last newline stay
    /// buffered for the next call.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(position) = self.buffer.iter().position(|&b| b == b'\n') {
            let mut raw: Vec<u8> = self.buffer.drain(..=position).collect();
            raw.pop();
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
            lines.push(String::from_utf8_lossy(&raw).into_owned());
        }
        lines
    }
}

// ADR-013: split `event_name:json.path` into its event-name prefix and
// the JSON path. Bare paths return ("", path); empty returns ("", "").
fn parse_stream_finish_path(p: &str) -> (&str, &str) {
    if p.is_empty() {
        return ("", "");
    }
    if let Some(idx) = p.find(':') {
        return (&p[..idx], &p[idx + 1..]);
    }
    ("", p)
}

/// The stream facts and stream locators for the effective wire shape. A shape
/// that declares its own (OpenAI Responses: event-typed SSE) outranks the
/// provider's, which describe the provider's default protocol (HANDOFF-087).
fn stream_settings(
    provider: &Provider,
    config: &ProviderSpec,
) -> Option<(&'static StreamDef, StreamLocators)> {
    let shape = config.chat_wire_shape;
    if let Some(stream) = wire_shape_stream_config(shape) {
        return Some((
            stream,
            StreamLocators {
                finish_reason: wire_shape_stream_finish_reason_path(shape),
                id: wire_shape_stream_response_id_path(shape),
                model: wire_shape_stream_response_model_path(shape),
            },
        ));
    }
    stream_config(provider.name).map(|stream| {
        (
            stream,
            StreamLocators {
                finish_reason: config.stream_finish_reason_path,
                id: config.stream_response_id_path,
                model: config.stream_response_model_path,
            },
        )
    })
}

fn build_stream_url(
    provider: &Provider,
    config: &ProviderSpec,
    stream: &StreamDef,
    model: &str,
) -> String {
    if stream.endpoint.is_empty() {
        return send_url(provider, config, &request_path(config, model));
    }

    let mut base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| config.base_url.to_string());
    if !config.region_env_var.is_empty() {
        if let Ok(region) = std::env::var(config.region_env_var) {
            base = base.replace("{region}", &region);
        }
    }

    let mut endpoint = stream.endpoint.replace("{model}", model);
    endpoint = endpoint.replace("{apiKey}", &provider.api_key);

    if matches!(auth_scheme(provider.name), AuthScheme::QueryParamKey) {
        let separator = if endpoint.contains('?') { "&" } else { "?" };
        endpoint.push_str(separator);
        endpoint.push_str(config.auth_query_param);
        endpoint.push('=');
        endpoint.push_str(&provider.api_key);
    }

    format!("{base}{endpoint}")
}

















































