use crate::structs::Response;
use reqwest::header::CONTENT_TYPE;
use serde_json::Value;

use crate::error::Error;
use crate::options::PromptOptions;
use crate::paths::{extract_string_path, opt_int_path};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::providers::generated::stream::{stream_config, StreamDef};
use crate::request::{build_request, build_url};
use crate::types::{Provider, Request, Usage};

pub async fn prompt_stream<F>(
    provider: &Provider,
    request: &Request,
    options: &PromptOptions,
    mut callback: F,
) -> Result<Response, Error>
where
    F: FnMut(&str),
{
    let stream = stream_config(provider.name).ok_or_else(|| {
        Error::Validation {
            field: "provider",
            message: format!("streaming not supported: {:?}", provider.name),
        }
    })?;

    let config = provider_config(provider.name);
    let url = build_stream_url(provider, config, stream);
    let msgs = crate::transforms::to_internal(&request.messages)?;
    let (mut body, headers) = build_request(provider, request, &msgs, options, &[])?;
    crate::caching::apply_caching(&mut body, provider, options, config).await?;
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

    let client = crate::http::shared_client();
    let mut request_builder = client
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .json(&body);
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

    let (finish_event, finish_json_path) = parse_stream_finish_path(config.stream_finish_reason_path);

    let mut usage = Usage::default();
    let mut full_text = String::new();
    let mut finish_reason: Option<String> = None;
    let mut current_event = String::new();
    let mut framer = SseFramer::default();
    let mut response = response;

    while let Some(chunk) = response.chunk().await? {
        for line in framer.push(&chunk)? {
            if let Some(event) = line.strip_prefix("event: ") {
                current_event = event.to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };

            // Data-level done sentinel (e.g., OpenAI [DONE]) is literal, not JSON.
            if !stream.done_signal.is_empty() && data == stream.done_signal {
                return Ok(Response {
                    text: full_text,
                    usage,
                    finish_reason: finish_reason.clone(),
                    ..Response::default()
                });
            }

            let parsed: Option<Value> = serde_json::from_str(data).ok();

            // ADR-013: capture stream-time finish-reason BEFORE the event-level
            // done return — Anthropic carries stop_reason on the message_stop
            // event body, which would otherwise be discarded.
            if let Some(ref parsed_value) = parsed {
                if !finish_json_path.is_empty()
                    && (finish_event.is_empty() || finish_event == current_event)
                {
                    let value = extract_string_path(parsed_value, finish_json_path);
                    if !value.is_empty() && value != "FINISH_REASON_UNSPECIFIED" {
                        finish_reason = Some(value);
                    }
                }
            }

            if stream.uses_event_types
                && !stream.done_event.is_empty()
                && current_event == stream.done_event
            {
                return Ok(Response {
                    text: full_text,
                    usage,
                    finish_reason: finish_reason.clone(),
                    ..Response::default()
                });
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

    Ok(Response {
        text: full_text,
        usage,
        finish_reason,
        ..Response::default()
    })
}

/// Frames an SSE byte stream into lines, then decodes each complete line.
///
/// Framing before decoding is the whole point (BUG-063). Provider chunk
/// boundaries are arbitrary, so a multibyte UTF-8 character regularly
/// straddles two chunks; decoding each chunk on its own turned both halves
/// into U+FFFD with no error. Bytes are buffered until a `\n` arrives, and
/// only a complete line is decoded, so a split character is whole by the time
/// it is read. A line that still is not UTF-8 is corrupt and fails loud.
#[derive(Default)]
pub(crate) struct SseFramer {
    buffer: Vec<u8>,
}

impl SseFramer {
    /// Append `chunk` and return every line it completes, in order, with the
    /// trailing `\n` and any `\r` removed. Bytes after the last newline stay
    /// buffered for the next call.
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, Error> {
        self.buffer.extend_from_slice(chunk);
        let mut lines = Vec::new();
        while let Some(position) = self.buffer.iter().position(|&b| b == b'\n') {
            let mut raw: Vec<u8> = self.buffer.drain(..=position).collect();
            raw.pop();
            if raw.last() == Some(&b'\r') {
                raw.pop();
            }
            let line = String::from_utf8(raw).map_err(|e| {
                Error::Stream(format!(
                    "invalid UTF-8 in event stream line at byte {}",
                    e.utf8_error().valid_up_to()
                ))
            })?;
            lines.push(line);
        }
        Ok(lines)
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

fn build_stream_url(provider: &Provider, config: &ProviderSpec, stream: &StreamDef) -> String {
    if stream.endpoint.is_empty() {
        return build_url(provider, config);
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

    // Both-empty is rejected by resolve_model at every entry point before
    // URL building runs, so the error arm is unreachable here.
    let model = crate::request::resolve_model(provider, config).unwrap_or_default();
    let mut endpoint = stream.endpoint.replace("{model}", &model);
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


















































