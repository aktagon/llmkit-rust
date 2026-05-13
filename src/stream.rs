use reqwest::header::CONTENT_TYPE;
use serde_json::Value;

use crate::error::Error;
use crate::options::PromptOptions;
use crate::paths::{extract_string_path, extract_u32_path};
use crate::providers::generated::providers::{provider_config, ProviderConfig};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::providers::generated::stream::{stream_config, StreamDef};
use crate::request::{build_request, build_url};
use crate::types::{Provider, Request, Response, Usage};

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
    let (mut body, headers) = build_request(provider, request, options)?;
    crate::caching::apply_caching(&mut body, provider, options, config).await?;
    if !stream.param.is_empty() {
        if let Some(object) = body.as_object_mut() {
            object.insert(stream.param.to_string(), Value::Bool(true));
        }
    }

    let client = reqwest::Client::new();
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

    let mut usage = Usage::default();
    let mut full_text = String::new();
    let mut current_event = String::new();
    let mut buffer = String::new();
    let mut response = response;

    while let Some(chunk) = response.chunk().await? {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(position) = buffer.find('\n') {
            let mut line = buffer[..position].to_string();
            buffer.drain(..=position);
            if line.ends_with('\r') {
                line.pop();
            }

            if let Some(event) = line.strip_prefix("event: ") {
                current_event = event.to_string();
                continue;
            }

            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };

            if !stream.done_signal.is_empty() && data == stream.done_signal {
                return Ok(Response {
                    text: full_text,
                    usage,
                    ..Response::default()
                });
            }

            if stream.uses_event_types && !stream.done_event.is_empty() && current_event == stream.done_event {
                return Ok(Response {
                    text: full_text,
                    usage,
                    ..Response::default()
                });
            }

            let Ok(parsed) = serde_json::from_str::<Value>(data) else {
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
                if current_event == stream.usage_event && !stream.usage_output_path.is_empty() {
                    usage.output = extract_u32_path(&parsed, stream.usage_output_path);
                    if !stream.usage_input_path.is_empty() {
                        usage.input = extract_u32_path(&parsed, stream.usage_input_path);
                    }
                }
            } else {
                let text = extract_string_path(&parsed, stream.delta_text_path);
                if !text.is_empty() {
                    full_text.push_str(&text);
                    callback(&text);
                }
                if !stream.usage_input_path.is_empty() {
                    let value = extract_u32_path(&parsed, stream.usage_input_path);
                    if value > 0 {
                        usage.input = value;
                    }
                }
                if !stream.usage_output_path.is_empty() {
                    let value = extract_u32_path(&parsed, stream.usage_output_path);
                    if value > 0 {
                        usage.output = value;
                    }
                }
            }

            current_event.clear();
        }
    }

    Ok(Response {
        text: full_text,
        usage,
        ..Response::default()
    })
}

fn build_stream_url(provider: &Provider, config: &ProviderConfig, stream: &StreamDef) -> String {
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

    let model = provider
        .model
        .clone()
        .unwrap_or_else(|| config.default_model.to_string());
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
