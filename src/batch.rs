use crate::structs::{BatchHandle, Response};
use serde_json::{json, Value};
use tokio::time::Duration;

use crate::error::Error;
use crate::http::{get_text, post_json, post_multipart};
use crate::job::{
    classify_by_config, non_empty_values, poll_job, Classification, JobAdapter, LifecycleConfig,
    PollBody,
};
use crate::middleware::{fire_post, fire_pre, set_event_error, Event, MiddlewareOp};
use crate::options::PromptOptions;
use crate::providers::generated::batch::{batch_config, BatchInputMode, BatchDef};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::request::{append_beta, build_auth_headers, build_request};
use crate::response::parse_response;
use crate::types::{Provider, Request};

///
///
#
pub struct BatchPoll {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for BatchPoll {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(2),
            timeout: Duration::from_secs(600),
        }
    }
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
        model: crate::request::resolve_model(provider, config)?,
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(&options.middleware, &base_event)?;

    let mws = options.middleware.clone();
    let outcome = submit_batch_inner(provider, requests, options, config).await;

    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &outcome {
        set_event_error(&mut post_event, err);
    }
    fire_post(&mws, &post_event);
    outcome
}

async fn submit_batch_inner(
    provider: &Provider,
    requests: &[Request],
    options: PromptOptions,
    config: &ProviderSpec,
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
    let mut headers = build_auth_headers(provider, config);

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
        BatchInputMode::InlineRequests => {
            let (payload, beta_headers) =
                build_batch_body(requests, provider, &options, config, batch).await?;
            //
            //
            //
            //
            for (k, v) in beta_headers {
                if k.eq_ignore_ascii_case("anthropic-beta") {
                    match headers
                        .iter_mut()
                        .find(|(hk, _)| hk.eq_ignore_ascii_case("anthropic-beta"))
                    {
                        Some((_, hv)) => *hv = append_beta(hv, &v),
                        None => headers.push((k, v)),
                    }
                } else {
                    headers.push((k, v));
                }
            }
            payload
        }
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

///
///
///
///
pub async fn wait_batch(
    handle: &BatchHandle,
    mut options: PromptOptions,
    poll: BatchPoll,
) -> Result<Vec<Response>, Error> {
    //
    //
    if handle.raw {
        options.raw = true;
    }
    let mut adapter = new_batch_adapter(handle, options.raw)?;
    //
    adapter.lc.poll_interval = poll.interval;
    adapter.lc.poll_timeout = poll.timeout;
    poll_job(&adapter).await
}

///
///
///
pub(crate) struct BatchAdapter {
    pub(crate) lc: LifecycleConfig,
    provider: Provider,
    base: String,
    headers: Vec<(String, String)>,
    batch: &'static BatchDef,
    lifecycle: &'static crate::ResourceLifecycleDef,
    poll_url: String,
    raw: bool,
}

impl JobAdapter for BatchAdapter {
    type Out = Vec<Response>;

    fn config(&self) -> &LifecycleConfig {
        &self.lc
    }

    async fn poll(&self) -> Result<PollBody, Error> {
        let (status, body) = get_text(&self.poll_url, &self.headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(
                &self.provider,
                status.as_u16(),
                &body,
            ));
        }
        let parsed: Value = serde_json::from_str(&body)?;
        Ok(PollBody::new(parsed))
    }

    fn classify(&self, body: &PollBody) -> Result<Classification, Error> {
        Ok(classify_by_config(&self.lc, body))
    }

    async fn result(&self, body: &PollBody) -> Result<Vec<Response>, Error> {
        //
        //
        //
        fetch_batch_results(
            &self.provider,
            &self.base,
            &self.headers,
            self.batch,
            self.lifecycle,
            &self.lc.id,
            self.raw,
            Some(body.value()),
        )
        .await
    }
}

///
///
///
///
///
pub(crate) fn new_batch_adapter(handle: &BatchHandle, raw: bool) -> Result<BatchAdapter, Error> {
    let provider = handle.provider.clone();
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
    let headers = build_auth_headers(&provider, config);
    let poll_url = if lifecycle.polling_endpoint.is_empty() {
        format!("{base}{}/{}", lifecycle.create_endpoint, handle.id)
    } else {
        format!(
            "{base}{}",
            lifecycle.polling_endpoint.replace("{id}", &handle.id)
        )
    };

    let defaults = BatchPoll::default();
    let lc = LifecycleConfig {
        noun: "batch",
        provider: format!("{:?}", provider.name),
        id: handle.id.clone(),
        status_path: lifecycle.polling_status_path.to_string(),
        done_values: non_empty_values([lifecycle.polling_done_value]),
        error_values: non_empty_values(lifecycle.polling_error_values.iter().copied()),
        error_message_path: String::new(),
        poll_interval: defaults.interval,
        poll_timeout: defaults.timeout,
    };
    Ok(BatchAdapter {
        lc,
        provider,
        base,
        headers,
        batch,
        lifecycle,
        poll_url,
        raw,
    })
}

///
///
///
///
///
async fn build_batch_body(
    requests: &[Request],
    provider: &Provider,
    options: &PromptOptions,
    config: &ProviderSpec,
    batch: &BatchDef,
) -> Result<(Value, Vec<(String, String)>), Error> {
    let mut items = Vec::new();
    let mut beta = String::new();
    for (index, request) in requests.iter().enumerate() {
        let msgs = crate::transforms::to_internal(&request.messages)?;
        let (mut body, req_headers) = build_request(provider, request, &msgs, options, &[])?;
        if let Some((_, v)) = req_headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("anthropic-beta"))
        {
            beta = append_beta(&beta, v);
        }
        //
        //
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

    let payload = if !batch.request_wrapper.is_empty() {
        json!({ batch.request_wrapper: items })
    } else {
        json!({ "requests": items })
    };
    let beta_headers = if beta.is_empty() {
        Vec::new()
    } else {
        vec![("anthropic-beta".to_string(), beta)]
    };
    Ok((payload, beta_headers))
}

async fn build_batch_jsonl(
    requests: &[Request],
    provider: &Provider,
    options: &PromptOptions,
    config: &ProviderSpec,
) -> Result<Vec<u8>, Error> {
    let batch = batch_config(provider.name).expect("batch config");
    let mut lines = String::new();
    for (index, request) in requests.iter().enumerate() {
        let msgs = crate::transforms::to_internal(&request.messages)?;
        let (mut body, _) = build_request(provider, request, &msgs, options, &[])?;
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
    status_raw: Option<&Value>,
) -> Result<Vec<Response>, Error> {
    let response_body = if !lifecycle.result_file_id_path.is_empty() {
        //
        //
        let parsed: Value = match status_raw {
            Some(value) => value.clone(),
            None => {
                let poll_url = format!("{}{}/{}", base, lifecycle.create_endpoint, handle_id);
                let (status, status_body) = get_text(&poll_url, headers).await?;
                if !status.is_success() {
                    return Err(crate::response::parse_api_error(
                        provider,
                        status.as_u16(),
                        &status_body,
                    ));
                }
                serde_json::from_str(&status_body)?
            }
        };
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
        //
        //
        //
        //
        let response_text = if batch.result_body_path.is_empty() {
            line.to_string()
        } else {
            let Ok(parsed) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(inner) = navigate_value_path(&parsed, batch.result_body_path) else {
                continue;
            };
            let Ok(text) = serde_json::to_string(inner) else {
                continue;
            };
            text
        };
        let Ok(mut resp) = parse_response(provider, &response_text) else {
            continue;
        };
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
