use serde_json::Value;

use crate::error::Error;
use crate::paths::{extract_string_path, extract_u32_path};
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::caching::cache_usage_paths;
use crate::{response_text_path, usage_paths, Provider, Response, Usage};

pub fn parse_response(provider: &Provider, body: &str) -> Result<Response, Error> {
    let raw: Value = serde_json::from_str(body)?;
    let text = extract_string_path(&raw, response_text_path(provider.name));
    let (input_path, output_path) = usage_paths(provider.name);
    let (creation_path, read_path) = cache_usage_paths(provider.name);

    Ok(Response {
        text,
        usage: Usage {
            input: extract_u32_path(&raw, input_path),
            output: extract_u32_path(&raw, output_path),
            cache_creation: extract_u32_path(&raw, creation_path),
            cache_read: extract_u32_path(&raw, read_path),
        },
    })
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
