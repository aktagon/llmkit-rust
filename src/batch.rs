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
use crate::providers::generated::batch::{
    batch_config, BatchDef, BatchInputMode, BATCH_REQUEST_ID_PREFIX, BATCH_SLOT_ERROR,
    BATCH_SLOT_MISSING,
};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::provider_turn::extract_raw_json_path;
use crate::request::{append_beta, build_auth_headers, build_request};
use crate::response::{attach_raw, decode_response_raw};
use crate::types::{Provider, Request};

/// Poll cadence for [`wait_batch`]. Defaults match Go (2s interval, 10min
/// timeout); tests override `interval` to run fast.
#[derive(Clone, Copy, Debug)]
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
            // The per-request bodies may require a contract-bearing anthropic-beta
            // (files-api / structured output) that build_auth_headers does not
            // set — ride it onto the batch CREATE request, else a file-referencing
            // batch item silently drops the beta (batch-modality witness family).
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

/// Polls the batch lifecycle until a terminal state and returns the ordered
/// responses. It is now a thin delegation to the shared job engine (ADR-062
/// §b) — [`poll_job`] owns the loop, deadline, and state machine; the
/// [`BatchAdapter`] carries the batch-specific seams. Signature byte-unchanged.
pub async fn wait_batch(
    handle: &BatchHandle,
    mut options: PromptOptions,
    poll: BatchPoll,
) -> Result<Vec<Response>, Error> {
    // ADR-014: a handle that remembers raw (from submit_batch or set by
    // a cross-process-resume caller) takes effect at wait time.
    if handle.raw {
        options.raw = true;
    }
    let mut adapter = new_batch_adapter(handle, options.raw)?;
    // The BatchPoll cadence (tests shrink it) drives the engine loop.
    adapter.lc.poll_interval = poll.interval;
    adapter.lc.poll_timeout = poll.timeout;
    poll_job(&adapter).await
}

/// Binds the batch capability to the job engine's four seams. It closes over
/// the resolved raw flag + provider config so `result` can perform batch's
/// two-hop (output_file_id -> GET /content) from the already-decoded poll body.
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
        // The poll body is already decoded — hand it to fetch_batch_results so
        // the two-hop provider (OpenAI: output_file_id lives in this same status
        // body) skips a redundant status GET (S1).
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

/// Assembles the batch adapter + its LifecycleConfig from the batch facts.
/// ErrorValues comes from the provider's `polling_error_values` fact (OpenAI:
/// failed/expired/cancelled); when absent (Anthropic — failures are per-request,
/// batch "ended" is done) it is empty and a stuck batch terminates at the
/// deadline backstop rather than mislabelling a Failed terminal.
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

/// Returns the batch payload plus the contract-bearing anthropic-beta values the
/// per-request bodies require (files-api / structured output), composed across
/// items, so the caller can attach them to the batch CREATE request
/// (build_request returns them per request; the batch submit otherwise sends only
/// auth headers).
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
        // Caching is a shared request-construction step (ADR-026), applied on
        // the batch path like Text/Agent.
        if options.caching {
            crate::caching::apply_caching(&mut body, provider, options, config).await?;
        }
        if !batch.item_body_field.is_empty() {
            items.push(json!({
                "custom_id": format!("{BATCH_REQUEST_ID_PREFIX}{index}"),
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
            "custom_id": format!("{BATCH_REQUEST_ID_PREFIX}{index}"),
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

/// Fetches and parses completed batch results.
///
/// A provider declares up to three result sources (HANDOFF-078): a direct
/// result endpoint (Anthropic), and file IDs in the status body for the output
/// file and the error file (OpenAI). Every source that is present is read, in
/// that order; the call fails only when none is. The status body also carries
/// the request count (`batch.request_count_paths`), which fixes the number of
/// result slots.
///
/// `status_raw` is the already-decoded poll body when the caller has it (the
/// poll engine does). When `None` and a file ID or the count is needed, the
/// status is fetched.
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
    let needs_status = !lifecycle.result_file_id_path.is_empty()
        || !lifecycle.error_file_id_path.is_empty()
        || !batch.request_count_paths.is_empty();
    let fetched_status;
    let status_body: Option<&Value> = match status_raw {
        Some(value) => Some(value),
        None if needs_status => {
            let poll_url = format!("{}{}/{}", base, lifecycle.create_endpoint, handle_id);
            let (status, body) = get_text(&poll_url, headers).await?;
            if !status.is_success() {
                return Err(crate::response::parse_api_error(
                    provider,
                    status.as_u16(),
                    &body,
                ));
            }
            fetched_status = serde_json::from_str::<Value>(&body)?;
            Some(&fetched_status)
        }
        None => None,
    };

    let mut sources = Vec::new();
    if !lifecycle.result_endpoint.is_empty() {
        let url = format!("{base}{}", lifecycle.result_endpoint.replace("{id}", handle_id));
        let (status, body) = get_text(&url, headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(provider, status.as_u16(), &body));
        }
        sources.push(body);
    }
    for id_path in [lifecycle.result_file_id_path, lifecycle.error_file_id_path] {
        if id_path.is_empty() {
            continue;
        }
        let file_id = status_body
            .map(|value| crate::paths::extract_string_path(value, id_path))
            .unwrap_or_default();
        if file_id.is_empty() {
            continue;
        }
        let url = format!(
            "{base}{}",
            lifecycle.file_content_endpoint.replace("{id}", &file_id)
        );
        let (status, body) = get_text(&url, headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(provider, status.as_u16(), &body));
        }
        sources.push(body);
    }
    if sources.is_empty() {
        return Err(Error::Unsupported(format!(
            "batch results: no result source for {:?} batch {handle_id}",
            provider.name
        )));
    }

    let count = status_body.and_then(|value| batch_request_count(value, batch.request_count_paths));
    Ok(parse_batch_results(provider, &sources, batch, raw, count))
}

/// Sums the integers at `paths` in the status body. `None` when no path
/// resolves to a number.
fn batch_request_count(status: &Value, paths: &[&str]) -> Option<usize> {
    let mut total: Option<usize> = None;
    for path in paths {
        if let Some(n) = crate::paths::opt_int_path(status, path) {
            total = Some(total.unwrap_or(0) + usize::try_from(n).unwrap_or(0));
        }
    }
    total
}

/// One parsed result line waiting for its index.
struct BatchSlot {
    response: Response,
    succeeded: bool,
}

/// Parses JSONL result sources into one [`Response`] per submitted request, at
/// that request's index (BUG-072, HANDOFF-078).
///
/// Providers return result lines in any order, so a line is placed by the
/// request id at `batch.result_key_path`: [`BATCH_REQUEST_ID_PREFIX`] + N goes
/// to index N. When one index appears twice, a line that succeeded replaces a
/// failed one, a failed line never replaces a succeeded one, and otherwise the
/// later line follows the indexed slots.
///
/// With a request count there are exactly `count` slots, and an id at or above
/// the count follows them. Without one, slots run to the highest index seen. An
/// index with no line reads [`BATCH_SLOT_MISSING`]. Lines whose id has another
/// form (a batch created outside llmkit) follow the indexed slots in file
/// order. A line that is not JSON cannot be placed and is skipped.
///
/// When `raw` is true, a succeeded Response carries its body (the unwrapped
/// inner body when `result_body_path` is set, otherwise the line); a failed
/// Response carries the whole line; a missing slot carries none.
fn parse_batch_results(
    provider: &Provider,
    sources: &[String],
    batch: &BatchDef,
    raw: bool,
    count: Option<usize>,
) -> Vec<Response> {
    let mut slots: Vec<Option<BatchSlot>> = Vec::new();
    if let Some(count) = count {
        slots.resize_with(count, || None);
    }
    let mut unkeyed = Vec::new();
    for data in sources {
        for line in data.lines().map(str::trim).filter(|line| !line.is_empty()) {
            let Ok(wrapper) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let slot = parse_batch_result_line(provider, line, &wrapper, batch, raw);

            let index = if batch.result_key_path.is_empty() {
                None
            } else {
                batch_request_index(&crate::paths::extract_string_path(
                    &wrapper,
                    batch.result_key_path,
                ))
            };
            let Some(index) = index.filter(|index| count.map_or(true, |count| *index < count)) else {
                unkeyed.push(slot.response);
                continue;
            };
            if slots.len() <= index {
                slots.resize_with(index + 1, || None);
            }
            match &slots[index] {
                None => slots[index] = Some(slot),
                Some(existing) if slot.succeeded && !existing.succeeded => {
                    slots[index] = Some(slot);
                }
                // The request succeeded; a failed duplicate adds nothing.
                Some(existing) if existing.succeeded && !slot.succeeded => {}
                Some(_) => unkeyed.push(slot.response),
            }
        }
    }

    let mut responses: Vec<Response> = slots
        .into_iter()
        .map(|slot| match slot {
            Some(slot) => slot.response,
            None => Response {
                finish_reason: Some(BATCH_SLOT_MISSING.to_string()),
                ..Response::default()
            },
        })
        .collect();
    responses.extend(unkeyed);
    responses
}

/// Decodes one result line. The line succeeded when the value at
/// `batch.result_status_path` is one of `batch.result_success_values` (any
/// value when the provider declares no status path) and its body decodes.
/// Every other line becomes a failed Response: empty text, the first reason
/// path that resolves as `finish_reason` ([`BATCH_SLOT_ERROR`] when none does)
/// and the first message path that resolves as `finish_message`.
fn parse_batch_result_line(
    provider: &Provider,
    line: &str,
    wrapper: &Value,
    batch: &BatchDef,
    raw: bool,
) -> BatchSlot {
    let signalled = batch.result_status_path.is_empty()
        || batch.result_success_values.contains(
            &crate::paths::extract_string_path(wrapper, batch.result_status_path).as_str(),
        );
    if signalled {
        let response_text = if batch.result_body_path.is_empty() {
            Some(line.to_string())
        } else {
            // VERBATIM, not parse-navigate-re-encode: the inner body is what
            // ADR-085 captures the assistant turn from, and serde's rendering
            // of a parsed value re-sorts object keys and reformats numbers.
            // Harmless while only scalars were read out of it; not harmless
            // once a payload is captured from the same bytes.
            extract_raw_json_path(line, batch.result_body_path)
        };
        if let Some(text) = response_text.filter(|text| text.starts_with('{')) {
            // Batch is Chat-Completions-only (ADR-055): an empty wire shape
            // selects the provider's declared response paths, not the
            // Responses output[] arm.
            if let Ok(response) = decode_response_raw(provider.name, "", &text, raw) {
                return BatchSlot {
                    response,
                    succeeded: true,
                };
            }
        }
    }

    let reason = first_path(wrapper, batch.result_reason_paths);
    let message = first_path(wrapper, batch.result_message_paths);
    let failed = Response {
        finish_reason: Some(if reason.is_empty() {
            BATCH_SLOT_ERROR.to_string()
        } else {
            reason
        }),
        finish_message: (!message.is_empty()).then_some(message),
        ..Response::default()
    };
    BatchSlot {
        response: attach_raw(failed, line, raw),
        succeeded: false,
    }
}

/// The value at the first path that resolves to a non-empty string, or ""
/// when none does.
fn first_path(data: &Value, paths: &[&str]) -> String {
    paths
        .iter()
        .map(|path| crate::paths::extract_string_path(data, path))
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

/// Reads N out of the [`BATCH_REQUEST_ID_PREFIX`] + N id the SDK sends with
/// request N. Any other id reports `None`.
fn batch_request_index(id: &str) -> Option<usize> {
    let digits = id.strip_prefix(BATCH_REQUEST_ID_PREFIX)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}















































































