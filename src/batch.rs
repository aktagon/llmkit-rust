use crate::structs::{BatchHandle, Response};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

use crate::error::Error;
use crate::http::{get_text, post_json, post_multipart};
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareOp};
use crate::options::PromptOptions;
use crate::providers::generated::batch::{batch_config, BatchInputMode, BatchDef};
use crate::providers::generated::providers::{provider_config, ProviderConfig};
use crate::request::{build_auth_headers, build_request};
use crate::response::parse_response;
use crate::types::{Provider, Request};

pub async fn prompt_batch(
    provider: &Provider,
    requests: &[Request],
    options: PromptOptions,
) -> Result<Vec<Response>, Error> {
    let handle = submit_batch(provider, requests, options.clone()).await?;
    wait_batch(&handle, options).await
}

pub async fn submit_batch(
    provider: &Provider,
    requests: &[Request],
    options: PromptOptions,
) -> Result<BatchHandle, Error> {
    crate::request::validate_provider(provider)?;

    let config = provider_config(provider.name);
    let base_event = Event {
        op: MiddlewareOp::BatchSubmit,
        provider: format!("{:?}", provider.name),
        model: provider
            .model
            .clone()
            .unwrap_or_else(|| config.default_model.to_string()),
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(&options.middleware, &base_event)?;

    let mws = options.middleware.clone();
    let outcome = submit_batch_inner(provider, requests, options, config).await;

    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &outcome {
        post_event.err = Some(err.to_string());
    }
    fire_post(&mws, &post_event);
    outcome
}

async fn submit_batch_inner(
    provider: &Provider,
    requests: &[Request],
    options: PromptOptions,
    config: &ProviderConfig,
) -> Result<BatchHandle, Error> {
    let batch = batch_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("batching not supported: {:?}", provider.name),
    })?;
    let lifecycle = batch.lifecycle.ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("async batching not supported: {:?}", provider.name),
    })?;

    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| config.base_url.to_string());
    let headers = build_auth_headers(provider, config);

    let body = match batch.input_mode {
        BatchInputMode::FileReferenceInput => {
            let jsonl = build_batch_jsonl(requests, provider, &options, config).await?;
            let file_id = upload_batch_file(&base, &headers, batch, jsonl).await?;
            json!({
                batch.input_field: file_id,
                "endpoint": batch.endpoint_path,
                "completion_window": batch.completion_window,
            })
        }
        BatchInputMode::InlineRequests => build_batch_body(requests, provider, &options, config, batch).await?,
    };

    let url = format!("{base}{}", lifecycle.create_endpoint);
    let (status, response_body) = post_json(&url, body, &headers).await?;
    if !status.is_success() {
        return Err(crate::response::parse_api_error(
            provider,
            status.as_u16(),
            &response_body,
        ));
    }
    let parsed: Value = serde_json::from_str(&response_body)?;
    let batch_id = crate::paths::extract_string_path(&parsed, lifecycle.response_id_path);
    if batch_id.is_empty() {
        return Err(Error::Unsupported("batch create: empty batch ID".into()));
    }
    Ok(BatchHandle {
        id: batch_id,
        provider: provider.clone(),
        raw: options.raw,
    })
}

pub async fn wait_batch(handle: &BatchHandle, mut options: PromptOptions) -> Result<Vec<Response>, Error> {
    // ADR-014: a handle that remembers raw (from submit_batch or set by
    // a cross-process-resume caller) takes effect at wait time.
    if handle.raw {
        options.raw = true;
    }
    let provider = &handle.provider;
    let config = provider_config(provider.name);
    let batch = batch_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("batching not supported: {:?}", provider.name),
    })?;
    let lifecycle = batch.lifecycle.ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("async batching not supported: {:?}", provider.name),
    })?;
    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| config.base_url.to_string());
    let headers = build_auth_headers(provider, config);

    let poll_url = if lifecycle.polling_endpoint.is_empty() {
        format!("{base}{}{}/{}", lifecycle.create_endpoint, "", handle.id)
    } else {
        format!("{base}{}", lifecycle.polling_endpoint.replace("{id}", &handle.id))
    };

    loop {
        let (status, response_body) = get_text(&poll_url, &headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(
                provider,
                status.as_u16(),
                &response_body,
            ));
        }
        let parsed: Value = serde_json::from_str(&response_body)?;
        let current = crate::paths::extract_string_path(&parsed, lifecycle.polling_status_path);
        if current == lifecycle.polling_done_value {
            return fetch_batch_results(
                provider,
                &base,
                &headers,
                batch,
                lifecycle,
                &handle.id,
                options.raw,
            )
            .await;
        }
        sleep(Duration::from_secs(2)).await;
    }
}

async fn build_batch_body(
    requests: &[Request],
    provider: &Provider,
    options: &PromptOptions,
    config: &ProviderConfig,
    batch: &BatchDef,
) -> Result<Value, Error> {
    let mut items = Vec::new();
    for (index, request) in requests.iter().enumerate() {
        let (mut body, _) = build_request(provider, request, options)?;
        // Caching is a shared request-construction step (ADR-026), applied on
        // the batch path like Text/Agent.
        if options.caching {
            crate::caching::apply_caching(&mut body, provider, options, config).await?;
        }
        if !batch.item_body_field.is_empty() {
            items.push(json!({
                "custom_id": format!("req-{index}"),
                batch.item_body_field: body,
            }));
        } else {
            items.push(body);
        }
    }

    if !batch.request_wrapper.is_empty() {
        Ok(json!({ batch.request_wrapper: items }))
    } else {
        Ok(json!({ "requests": items }))
    }
}

async fn build_batch_jsonl(
    requests: &[Request],
    provider: &Provider,
    options: &PromptOptions,
    config: &ProviderConfig,
) -> Result<Vec<u8>, Error> {
    let batch = batch_config(provider.name).expect("batch config");
    let mut lines = String::new();
    for (index, request) in requests.iter().enumerate() {
        let (mut body, _) = build_request(provider, request, options)?;
        if options.caching {
            crate::caching::apply_caching(&mut body, provider, options, config).await?;
        }
        let line = json!({
            "custom_id": format!("req-{index}"),
            "method": "POST",
            "url": batch.endpoint_path,
            "body": body,
        });
        lines.push_str(&serde_json::to_string(&line)?);
        lines.push('\n');
    }
    Ok(lines.into_bytes())
}

async fn upload_batch_file(
    base: &str,
    headers: &[(String, String)],
    batch: &BatchDef,
    data: Vec<u8>,
) -> Result<String, Error> {
    let form = reqwest::multipart::Form::new()
        .text("purpose", batch.file_purpose.to_string())
        .part(
            "file",
            reqwest::multipart::Part::bytes(data).file_name("batch_input.jsonl"),
        );
    let url = format!("{base}/v1/files");
    let (status, response_body) = post_multipart(&url, form, headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: "batch_file_upload".into(),
            status_code: status.as_u16(),
            message: response_body,
        });
    }
    let parsed: Value = serde_json::from_str(&response_body)?;
    let file_id = crate::paths::extract_string_path(&parsed, "id");
    if file_id.is_empty() {
        return Err(Error::Unsupported("batch file upload: empty file ID".into()));
    }
    Ok(file_id)
}

async fn fetch_batch_results(
    provider: &Provider,
    base: &str,
    headers: &[(String, String)],
    batch: &BatchDef,
    lifecycle: &crate::ResourceLifecycleDef,
    handle_id: &str,
    raw: bool,
) -> Result<Vec<Response>, Error> {
    let response_body = if !lifecycle.result_file_id_path.is_empty() {
        let poll_url = format!("{}{}/{}", base, lifecycle.create_endpoint, handle_id);
        let (status, status_body) = get_text(&poll_url, headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(
                provider,
                status.as_u16(),
                &status_body,
            ));
        }
        let parsed: Value = serde_json::from_str(&status_body)?;
        let file_id = crate::paths::extract_string_path(&parsed, lifecycle.result_file_id_path);
        if file_id.is_empty() {
            return Err(Error::Unsupported("batch results: empty output file ID".into()));
        }
        let url = format!(
            "{base}{}",
            lifecycle.file_content_endpoint.replace("{id}", &file_id)
        );
        let (status, body) = get_text(&url, headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(provider, status.as_u16(), &body));
        }
        body
    } else if !lifecycle.result_endpoint.is_empty() {
        let url = format!("{base}{}", lifecycle.result_endpoint.replace("{id}", handle_id));
        let (status, body) = get_text(&url, headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(provider, status.as_u16(), &body));
        }
        body
    } else {
        return Err(Error::Unsupported(format!(
            "batch result endpoint not configured for {:?}",
            provider.name
        )));
    };

    parse_batch_results(provider, &response_body, batch, raw)
}

fn parse_batch_results(
    provider: &Provider,
    data: &str,
    batch: &BatchDef,
    raw: bool,
) -> Result<Vec<Response>, Error> {
    let mut responses = Vec::new();
    for line in data.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let response_text = if batch.result_body_path.is_empty() {
            line.to_string()
        } else {
            let parsed: Value = serde_json::from_str(line)?;
            let inner = navigate_value_path(&parsed, batch.result_body_path)
                .ok_or_else(|| Error::Unsupported("batch result wrapper missing body path".into()))?;
            serde_json::to_string(inner)?
        };
        let mut resp = parse_response(provider, &response_text)?;
        if raw {
            resp.raw = serde_json::from_str(&response_text).ok();
        }
        responses.push(resp);
    }
    Ok(responses)
}

fn navigate_value_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}
