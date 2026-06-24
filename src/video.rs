//! Video generation runtime — mirror of go/video.go (ADR-034).
//!
//! Asynchronous text-to-video: `submit_video` returns a [`VideoHandle`]
//! immediately; poll it with `VideoHandle::wait` (defined as an extension
//! trait in `builders/video.rs`). Pre-flight validation rejects unknown
//! models, lyrics parts, and image-to-video before any HTTP call.
//!
//! Dispatch branches on the provider config's `wire_shape` — never on the
//! provider name. An unknown shape is rejected at both the submit and poll
//! seams (not defaulted to Grok). Wired shapes:
//!
//!   - VideoGrok: POST {model, prompt} to gen_endpoint; submit response is
//!     {"request_id": "..."}. Poll GET {base}/v1/videos/{id} until
//!     status=="done" → video.{url, duration} (url delivery, no download).
//!   - VideoZhipu: POST {model, prompt}; submit response carries the poll
//!     handle as the top-level "id". Poll GET {base}/v4/async-result/{id}
//!     until task_status=="SUCCESS" → video_result[0].url (url delivery).
//!   - VideoTogether: POST {model, prompt}; submit response carries the poll
//!     handle as the top-level "id". Poll GET {base}/v2/videos/{id} until
//!     status=="completed" → outputs.video_url (url delivery).
//!   - VideoQwen: POST {model, input:{prompt}} with the required
//!     X-DashScope-Async: enable header; submit response carries the poll
//!     handle at output.task_id (dotted path). Poll GET {base}/api/v1/tasks/{id}
//!     until output.task_status=="SUCCEEDED" → output.video_url (url delivery).
//!
//! `submit_video` fires the `VideoGeneration` middleware op pre + post
//! around the HTTP submit (mirroring batch-submit semantics — never around
//! the wait poll loop). Mirrors music's `generate_music` fire pattern.

use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;

use crate::error::Error;
use crate::http::{get_bytes, get_text, get_text_sigv4, post_json, post_json_sigv4};
use crate::image::Part;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::providers::generated::video_gen::{video_gen_config, VideoGenDef, VideoModelDef};
use crate::request::{build_auth_headers, validate_provider};
use crate::structs::{MediaRef, VideoData, VideoHandle, VideoResponse};
use crate::types::Provider;

// Default poll cadence for VideoHandle::wait. xAI documents up-to-several-
// minute generations; the runtime polls every interval until timeout
// elapses (ADR-034 D2; per-call overrides deferred). Exposed as a
// configurable struct so tests can shrink the interval (mirrors Go's
// package vars).
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(600);

/// Video-generation request (ADR-034).
///
/// Model is required: video-generation models are explicit choices and the
/// text-generation default does not generate video.
///
/// Input is provided in one of two mutually-exclusive forms:
///   - `prompt`: terse sugar for the prompt-only hot path. Internally
///     desugars to `parts: vec![Part::text(prompt)]` before serialisation.
///   - `parts`: canonical sequence of text parts (slice 1 is text-to-video).
///
/// Pre-flight validation requires exactly one of `prompt` or `parts` to be
/// non-empty (XOR).
#[derive(Clone, Debug, Default)]
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    pub parts: Vec<Part>,

    /// Caller-supplied destination S3 URI for output-uri delivery providers
    /// (Bedrock Nova Reel writes the mp4 to the caller's own S3 bucket).
    /// Required when the provider's config sets `requires_output_uri`;
    /// ignored otherwise. Set it on the builder via `Video::output_uri`.
    pub output_uri: String,
}

/// Poll cadence for [`VideoHandle::wait`]. Defaults match Go
/// (5s interval, 10min timeout); tests override `interval` to run fast.
#[derive(Clone, Copy, Debug)]
pub struct VideoPoll {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for VideoPoll {
    fn default() -> Self {
        Self {
            interval: DEFAULT_POLL_INTERVAL,
            timeout: DEFAULT_POLL_TIMEOUT,
        }
    }
}

/// Submits an asynchronous text-to-video job and returns a [`VideoHandle`]
/// immediately. Poll the handle with `wait`. Pre-flight validation rejects
/// unknown models and unsupported part kinds before any HTTP call. Fires the
/// `VideoGeneration` middleware op pre + post around the HTTP submit (not
/// around the wait poll loop — batch-submit semantics).
pub async fn submit_video(
    provider: &Provider,
    request: &VideoRequest,
    middleware: &[MiddlewareFn],
    raw: bool,
) -> Result<VideoHandle, Error> {
    validate_provider(provider)?;
    if request.model.is_empty() {
        return Err(Error::Validation {
            field: "model",
            message: "required for video generation".into(),
        });
    }

    let parts = normalize_video_parts(request)?;

    let vg_cfg = video_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support video generation", provider.name),
    })?;
    let model = find_video_model(vg_cfg, &request.model).ok_or_else(|| Error::Validation {
        field: "model",
        message: format!(
            "{} is not a known video-generation model for {:?}",
            request.model, provider.name
        ),
    })?;

    for part in &parts {
        match part {
            Part::Lyrics(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "video generation does not accept lyrics parts".into(),
                });
            }
            Part::Image(_) => {
                // Image-to-video seed frame (BUG-010): accepted only by models
                // whose VideoModelDef sets supports_image_to_video;
                // text-to-video-only models reject it pre-flight rather than
                // silently dropping it at wire time.
                if !model.supports_image_to_video {
                    return Err(Error::Validation {
                        field: "parts",
                        message: format!(
                            "{} is a text-to-video-only model and does not accept image parts",
                            request.model
                        ),
                    });
                }
            }
            Part::Text(s) if s.is_empty() => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "must have text set".into(),
                });
            }
            Part::Text(_) => {}
        }
    }
    // VID-005: output-uri providers (Bedrock Nova Reel) write the video to the
    // caller's own S3 bucket, so the submit MUST carry a destination URI.
    // Reject pre-flight rather than letting the provider 400.
    if vg_cfg.requires_output_uri && request.output_uri.is_empty() {
        return Err(Error::Validation {
            field: "output_uri",
            message: format!(
                "{:?} requires a caller output S3 URI; set output_uri on the request",
                provider.name
            ),
        });
    }

    let cfg = provider_config(provider.name);
    let base = video_base_url(provider, cfg, vg_cfg);
    let mut headers = build_auth_headers(provider, cfg);
    headers.push(("content-type".into(), "application/json".into()));

    let base_event = Event {
        op: MiddlewareOp::VideoGeneration,
        provider: format!("{:?}", provider.name),
        model: request.model.clone(),
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(middleware, &base_event)?;

    let result = dispatch_video_submit(
        provider,
        cfg,
        vg_cfg,
        &base,
        &headers,
        &request.model,
        &request.output_uri,
        &parts,
    )
    .await;

    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &result {
        post_event.err = Some(err.to_string());
    }
    fire_post(middleware, &post_event);

    let request_id = result?;

    Ok(VideoHandle {
        id: request_id,
        provider: provider.clone(),
        raw,
        // Carried so wait can build a model-templated poll URL (Vertex Veo
        // polls POST /{model}:fetchPredictOperation). Empty-ignored by every
        // other shape's poll endpoint.
        model: request.model.clone(),
    })
}

/// POSTs the submit body per wire shape (never by provider name) and
/// returns the provider-assigned poll handle id.
///
///   - VideoGrok (xAI), VideoZhipu (CogVideoX), and VideoTogether share the
///     simple {model, prompt} submit body. They differ only in which response
///     field carries the poll handle: Grok returns it as request_id, Zhipu and
///     Together as the top-level id.
///   - VideoQwen (DashScope) nests the prompt under an `input` object
///     ({model, input:{prompt}}) and requires the X-DashScope-Async: enable
///     header.
///   - VideoBedrock (Nova Reel) carries the model in the BODY (modelId), nests
///     the prompt under modelInput, carries the caller S3 URI under
///     outputDataConfig, and is signed with SigV4 (the bearer/query-param
///     header map does not apply).
///
/// The body and any per-shape headers are selected by wire shape; the poll
/// handle id is always read from the config-declared dotted path (OQ7).
async fn dispatch_video_submit(
    provider: &Provider,
    cfg: &ProviderSpec,
    vg_cfg: &VideoGenDef,
    base: &str,
    headers: &[(String, String)],
    model: &str,
    output_uri: &str,
    parts: &[Part],
) -> Result<String, Error> {
    // Submit endpoint from the config-declared base + relative path (Option D);
    // handle id from the config-declared dotted path (OQ7).
    let (body, post_headers) = if vg_cfg.wire_shape == "VideoQwen" {
        // DashScope's async submit requires this header; set per-request only
        // so it never leaks into the shared auth-header map.
        let mut h = headers.to_vec();
        h.push(("X-DashScope-Async".to_string(), "enable".to_string()));
        (
            json!({
                "model": model,
                "input": { "prompt": join_prompt_text(parts) },
            }),
            h,
        )
    } else if vg_cfg.wire_shape == "VideoVeo" || vg_cfg.wire_shape == "VideoVertexVeo" {
        // Veo (Gemini API) and Vertex Veo share the submit body: the model is in
        // the submit PATH (:predictLongRunning), not the body — so the body has
        // no model field. The prompt nests under instances[]; the optional
        // parameters object is omitted on the prompt-only hot path.
        (
            json!({
                "instances": [{ "prompt": join_prompt_text(parts) }],
            }),
            headers.to_vec(),
        )
    } else if vg_cfg.wire_shape == "VideoBedrock" {
        // Nova Reel carries the model in the BODY (modelId, unlike the Converse
        // chat path) and writes the mp4 to the caller's S3 bucket. The optional
        // videoGenerationConfig {durationSeconds, fps, dimension, seed} is
        // omitted on the prompt-only hot path (provider defaults apply).
        (
            json!({
                "modelId": model,
                "modelInput": {
                    "taskType": "TEXT_VIDEO",
                    "textToVideoParams": { "text": join_prompt_text(parts) },
                },
                "outputDataConfig": {
                    "s3OutputDataConfig": { "s3Uri": output_uri },
                },
            }),
            headers.to_vec(),
        )
    } else {
        // Image-to-video (BUG-010): when a seed frame is present (only reachable
        // for grok-imagine-video, the lone supports_image_to_video model this
        // slice), inline it as a data URL in xAI's image.url field — the same
        // encoding the Grok image-edit path uses. Absent on the text-to-video
        // hot path, so the existing video-grok golden is unchanged.
        let mut b = json!({
            "model": model,
            "prompt": join_prompt_text(parts),
        });
        if let Some(seed) = video_seed_image_url(parts)? {
            b["image"] = json!({ "url": seed });
        }
        (b, headers.to_vec())
    };
    // {model} in the submit endpoint is substituted with the per-call model
    // (Veo's :predictLongRunning path); a no-op for providers that carry the
    // model in the body. Query-param auth (Google ?key=) is appended last.
    let url = append_video_auth(
        &format!("{base}{}", vg_cfg.gen_endpoint.replace("{model}", model)),
        provider,
        cfg,
    );
    // Bedrock signs every request (SigV4); the bearer/query-param header map
    // does not apply. Region/secret/session come from the AWS env vars (empty
    // when unset — like Go's os.Getenv — so the keyless test path still signs).
    let (status, response_body) = if matches!(auth_scheme(provider.name), AuthScheme::SigV4) {
        let (region, secret_key, session_token) = sigv4_env(cfg);
        post_json_sigv4(
            &url,
            body,
            &provider.api_key,
            &secret_key,
            &session_token,
            &region,
            cfg.service_name,
        )
        .await?
    } else {
        post_json(&url, body, &post_headers).await?
    };
    if !status.is_success() {
        return Err(Error::Api {
            provider: "video_submit".into(),
            status_code: status.as_u16(),
            message: response_body,
        });
    }
    let raw: Value = serde_json::from_str(&response_body)?;
    let id = lookup_handle_field(&raw, vg_cfg.submit_handle_field);
    if id.is_empty() {
        return Err(Error::Unsupported(format!(
            "video submit: empty handle field {:?}",
            vg_cfg.submit_handle_field
        )));
    }
    Ok(id)
}

/// Polls the provider until the video job reaches a terminal state, then
/// returns the finished [`VideoResponse`]. A failed or expired job surfaces
/// as an error. The handle carries the request id and provider config, so
/// wait works across process boundaries. `poll` is configurable so tests
/// can shrink the interval (mirrors Go's package vars).
pub async fn wait_video(handle: &VideoHandle, poll: VideoPoll) -> Result<VideoResponse, Error> {
    let provider = &handle.provider;
    let cfg = provider_config(provider.name);
    let vg_cfg = video_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support video generation", provider.name),
    })?;

    let base = video_base_url(provider, cfg, vg_cfg);
    let headers = build_auth_headers(provider, cfg);

    // Poll dispatch has three arms, selected here once before the loop:
    //   - sigv4 (Bedrock): signs the poll GET and carries the handle ARN as a
    //     single percent-encoded path segment (its ':' and '/' must not split
    //     into extra segments).
    //   - vertex_poll (Vertex Veo): the ONLY POST-poll shape — fetches the
    //     operation with a POST to {model}:fetchPredictOperation carrying
    //     {operationName}. The model is templated from the handle; the operation
    //     name goes in the body, not the URL.
    //   - default: the verbatim {id} substitution and a GET on the bearer/
    //     query-param auth path (every other provider).
    //
    // The arms are config-disjoint by design: sigv4 keys off AuthScheme and
    // vertex_poll off wire_shape, and no A-Box pairs SigV4 with VideoVertexVeo
    // (Bedrock is SigV4+VideoBedrock; Vertex is bearer+VideoVertexVeo). sigv4 is
    // matched first so a hypothetical both-true misconfig would poll as SigV4.
    let sigv4 = matches!(auth_scheme(provider.name), AuthScheme::SigV4);
    let vertex_poll = !sigv4 && vg_cfg.wire_shape == "VideoVertexVeo";
    let (poll_url, vertex_poll_body) = if sigv4 {
        // path_escape_arn encodes the ARN's '/' to %2F (keeping it a single
        // path segment) but leaves ':' literal — which matches Bedrock's SigV4
        // canonicalization: the live-verified Converse chat path signs a model
        // id carrying ':' literally and AWS accepts it, so ':' is not
        // %3A-encoded. The signer canonicalizes Url::path() (which preserves
        // the %2F), so the signed path equals the wire path. (Poll signing
        // itself is NOT live-anchored — no AWS key.)
        (
            format!(
                "{base}{}",
                vg_cfg
                    .poll_endpoint
                    .replace("{id}", &path_escape_arn(&handle.id))
            ),
            None,
        )
    } else if vertex_poll {
        // Vertex polls with a POST to {model}:fetchPredictOperation; the model
        // is templated into the path, the operation name (the handle id) goes
        // in the JSON body. Query-param auth is a no-op for Vertex (bearer).
        let url = append_video_auth(
            &format!("{base}{}", vg_cfg.poll_endpoint.replace("{model}", &handle.model)),
            provider,
            cfg,
        );
        let body = json!({ "operationName": handle.id });
        (url, Some(body))
    } else {
        (
            append_video_auth(
                &video_poll_url(vg_cfg.poll_endpoint, &base, &handle.id),
                provider,
                cfg,
            ),
            None,
        )
    };

    let deadline = std::time::Instant::now() + poll.timeout;
    loop {
        if std::time::Instant::now() > deadline {
            return Err(Error::Unsupported(format!(
                "video poll: timed out waiting for {}",
                handle.id
            )));
        }

        let (status, response_body) = if sigv4 {
            let (region, secret_key, session_token) = sigv4_env(cfg);
            get_text_sigv4(
                &poll_url,
                &provider.api_key,
                &secret_key,
                &session_token,
                &region,
                cfg.service_name,
            )
            .await?
        } else if let Some(body) = &vertex_poll_body {
            // Vertex Veo is the only POST-poll shape (fetchPredictOperation).
            post_json(&poll_url, body.clone(), &headers).await?
        } else {
            get_text(&poll_url, &headers).await?
        };
        if !status.is_success() {
            return Err(Error::Api {
                provider: "video_poll".into(),
                status_code: status.as_u16(),
                message: response_body,
            });
        }

        let (resp, done) = parse_video_poll(vg_cfg, &response_body)?;
        if done {
            // Two-hop providers (vg_cfg.file_endpoint set, e.g. minimax): the
            // terminal poll carried a file reference, not a video URL — resolve
            // it with one more GET before returning.
            let mut final_resp = if !vg_cfg.file_endpoint.is_empty() {
                resolve_video_file(&base, vg_cfg, &response_body, &headers).await?
            } else {
                resp
            };
            // Delivery dispatch (VID-005). Download-delivery providers (Veo)
            // returned a temporary fetch URI in VideoData.url; GET it and fill
            // VideoData.bytes (clearing url, per the source-XOR contract). Url-
            // and output-uri-delivery providers leave the url.
            if vg_cfg.output_delivery == "DeliveryDownload" {
                final_resp = download_video_bytes(provider, cfg, final_resp).await?;
            }
            if handle.raw {
                final_resp.raw = serde_json::from_str(&response_body).ok();
            }
            return Ok(final_resp);
        }

        tokio::time::sleep(poll.interval).await;
    }
}

/// Resolves the base for the video API (Option D): an explicit per-client
/// override wins (tests point it at a mock; users at a proxy), else the
/// provider's distinct video base (vg_cfg.video_base_url) when the video host
/// differs from chat, else the chat base. Endpoints are always relative paths
/// joined to this base — never absolute — so the host stays overridable.
fn video_base_url(provider: &Provider, cfg: &ProviderSpec, vg_cfg: &VideoGenDef) -> String {
    if let Some(b) = &provider.base_url {
        return b.clone();
    }
    let mut base = if !vg_cfg.video_base_url.is_empty() {
        vg_cfg.video_base_url.to_string()
    } else {
        cfg.base_url.to_string()
    };
    // SigV4 hosts carry a {region} placeholder (Bedrock:
    // bedrock-runtime.{region}.amazonaws.com) resolved from the region env var;
    // a no-op for every provider without the placeholder.
    if !cfg.region_env_var.is_empty() {
        let region = std::env::var(cfg.region_env_var).unwrap_or_default();
        base = base.replace("{region}", &region);
    }
    base
}

/// Substitutes {id} in the config poll template (an A-Box fact, OQ7) and joins
/// it to the resolved video base.
fn video_poll_url(poll_endpoint: &str, base: &str, id: &str) -> String {
    format!("{base}{}", poll_endpoint.replace("{id}", id))
}

/// Descends a dotted path (e.g. "id", "output.task_id") through the decoded
/// submit response, returning the string leaf or "" if a segment is missing
/// or the leaf is not a string.
fn lookup_handle_field(raw: &Value, path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let mut cur = raw;
    for seg in path.split('.') {
        match cur.get(seg) {
            Some(v) => cur = v,
            None => return String::new(),
        }
    }
    cur.as_str().unwrap_or("").to_string()
}

/// Decodes one poll response. Returns (resp, done):
///   - done=false when the job is still pending (caller keeps polling).
///   - done=true with the finished VideoResponse when terminal-success.
///   - Err when the job failed or expired.
///
///   - VideoGrok: {"status": "...", "video": {"url", "duration"}} or
///     {"status": "failed", "error": {"code", "message"}}.
///   - VideoZhipu: {"task_status": "SUCCESS"|"FAIL"|"PROCESSING",
///     "video_result": [{"url"}]}.
///   - VideoTogether: {"status": "completed"|"failed"|"cancelled"|"queued"|
///     "in_progress", "outputs": {"video_url"}}.
///   - VideoQwen: {"output": {"task_status": "SUCCEEDED"|"FAILED"|"CANCELED"|
///     "PENDING"|"RUNNING"|"UNKNOWN", "video_url"}}.
fn parse_video_poll(vg_cfg: &VideoGenDef, body: &str) -> Result<(VideoResponse, bool), Error> {
    let raw: Value = serde_json::from_str(body)?;

    // Unknown shape rejected (not defaulted to Grok): a forgotten poll arm
    // fails loud instead of hanging on a never-terminal status.
    match vg_cfg.wire_shape {
        "VideoQwen" => {
            let status = raw
                .get("output")
                .and_then(|o| o.get("task_status"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match status {
                "SUCCEEDED" => Ok((video_result_from_qwen(vg_cfg, &raw), true)),
                "FAILED" | "CANCELED" => Err(Error::Unsupported(format!(
                    "video generation {status}"
                ))),
                // PENDING, RUNNING, UNKNOWN (or any non-terminal status)
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoTogether" => {
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "completed" => Ok((video_result_from_together(vg_cfg, &raw), true)),
                "failed" | "cancelled" => Err(Error::Unsupported(format!(
                    "video generation {status}"
                ))),
                // queued, in_progress (or any non-terminal status)
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoZhipu" => {
            let status = raw
                .get("task_status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match status {
                "SUCCESS" => Ok((video_result_from_zhipu(vg_cfg, &raw), true)),
                "FAIL" => Err(Error::Unsupported("video generation failed".into())),
                // PROCESSING (or any non-terminal status)
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoVidu" => {
            // Vidu (Shengshu) task poll: state success terminal-success, failed
            // terminal-error, created/queueing/processing pending. The finished
            // video URL sits at creations[0].url (url delivery, single-hop).
            let state = raw.get("state").and_then(|v| v.as_str()).unwrap_or("");
            match state {
                "success" => Ok((video_result_from_vidu(vg_cfg, &raw), true)),
                "failed" => {
                    let msg = raw
                        .get("err_code")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .or_else(|| {
                            raw.get("message")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                        })
                        .unwrap_or("operation failed");
                    Err(Error::Unsupported(format!(
                        "video generation failed: {msg}"
                    )))
                }
                // created, queueing, processing (or any non-terminal state)
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoMinimax" => {
            // Two-hop: terminal-success yields a file_id, not a URL. Report
            // done with an empty result; wait_video performs the file-retrieve
            // hop (gated on vg_cfg.file_endpoint) and fills the URL.
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "Success" => Ok((VideoResponse::default(), true)),
                "Fail" => Err(Error::Unsupported("video generation failed".into())),
                // Queueing, Preparing, Processing (or any non-terminal status)
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoVeo" => {
            // Operation-based LRO: poll until done==true (the long-running-
            // operation done flag, not a status string). A done op carrying an
            // error object is a terminal failure; otherwise the response holds
            // the finished video.
            let done = raw.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
            if !done {
                return Ok((VideoResponse::default(), false));
            }
            if let Some(err_obj) = raw.get("error").filter(|e| e.is_object()) {
                let msg = err_obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("operation failed");
                return Err(Error::Unsupported(format!(
                    "video generation failed: {msg}"
                )));
            }
            // A done op with neither error nor a usable uri must surface as an
            // error, not a silent zero-byte success: download delivery would
            // otherwise GET nothing and return a VideoData with empty bytes and
            // empty url.
            let result = video_result_from_veo(vg_cfg, &raw);
            if result.videos.first().map(|v| v.url.is_empty()).unwrap_or(true) {
                return Err(Error::Unsupported(
                    "video generation: operation done but carried no video uri".into(),
                ));
            }
            Ok((result, true))
        }
        "VideoVertexVeo" => {
            // Vertex Veo operation poll (fetchPredictOperation): same done/error
            // LRO shape as Gemini Veo, but the finished video arrives as inline
            // base64 in the poll body (response.videos[0].bytesBase64Encoded),
            // not a fetch URI.
            let done = raw.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
            if !done {
                return Ok((VideoResponse::default(), false));
            }
            if let Some(err_obj) = raw.get("error").filter(|e| e.is_object()) {
                let msg = err_obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("operation failed");
                return Err(Error::Unsupported(format!(
                    "video generation failed: {msg}"
                )));
            }
            let result = video_result_from_vertex_veo(vg_cfg, &raw)?;
            // Mirror the Veo done+no-uri guard: a done op carrying no decodable
            // bytes must surface as an error, not a silent zero-byte success.
            if result.videos.first().map(|v| v.bytes.is_empty()).unwrap_or(true) {
                return Err(Error::Unsupported(
                    "video generation: operation done but carried no video bytes".into(),
                ));
            }
            Ok((result, true))
        }
        "VideoBedrock" => {
            // Bedrock async-invoke status (GetAsyncInvoke): Completed terminal-
            // success, Failed terminal-error (failureMessage), InProgress
            // pending. On success the provider wrote the mp4 to the caller's S3
            // bucket and echoes the URI.
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "Completed" => {
                    // A Completed invocation that echoes no output s3 uri must
                    // surface as an error, not a silent empty success (mirrors
                    // the Veo done+no-uri guard): the caller would otherwise get
                    // a "successful" VideoResponse whose URL is empty and never
                    // find the mp4.
                    let result = video_result_from_bedrock(vg_cfg, &raw);
                    if result.videos.first().map(|v| v.url.is_empty()).unwrap_or(true) {
                        return Err(Error::Unsupported(
                            "video generation: completed but carried no output s3 uri".into(),
                        ));
                    }
                    Ok((result, true))
                }
                "Failed" => {
                    let msg = raw
                        .get("failureMessage")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or("operation failed");
                    Err(Error::Unsupported(format!(
                        "video generation failed: {msg}"
                    )))
                }
                // InProgress (or any non-terminal status)
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoGrok" => {
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "done" => Ok((video_result_from_grok(vg_cfg, &raw), true)),
                "failed" | "expired" => {
                    let mut msg = status.to_string();
                    if let Some(m) = raw
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        msg = m.to_string();
                    }
                    Err(Error::Unsupported(format!(
                        "video generation {status}: {msg}"
                    )))
                }
                // pending (or any non-terminal status)
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        other => Err(Error::Unsupported(format!(
            "video poll: unsupported wire shape {other:?}"
        ))),
    }
}

/// Extracts the finished video from a Grok poll response. Grok uses url
/// delivery: VideoData.url carries a temporary xAI-hosted URL and bytes
/// stays empty (the SDK does not download on the caller's behalf).
fn video_result_from_grok(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let video = match raw.get("video") {
        Some(v) if v.is_object() => v,
        _ => return VideoResponse::default(),
    };
    let url = video
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let duration_seconds = video
        .get("duration")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds,
        }],
        ..VideoResponse::default()
    }
}

/// Extracts the finished video from a Zhipu CogVideoX poll response. Zhipu
/// uses url delivery: the finished video sits at video_result[0].url (no
/// duration field on the result), so VideoData.url carries the temporary
/// Zhipu-hosted URL and bytes stays empty.
fn video_result_from_zhipu(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("video_result")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|first| first.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return VideoResponse::default();
    }
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    }
}

/// Extracts the finished video from a Vidu (Shengshu) poll response. Vidu
/// uses url delivery: the finished video sits at creations[0].url, so
/// VideoData.url carries the temporary Vidu-hosted URL and bytes stays empty.
fn video_result_from_vidu(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("creations")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|first| first.get("url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return VideoResponse::default();
    }
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    }
}

/// Extracts the finished video from a Together poll response. Together uses
/// url delivery: the finished video sits at outputs.video_url, so
/// VideoData.url carries the temporary Together-hosted URL and bytes stays
/// empty.
fn video_result_from_together(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("outputs")
        .and_then(|o| o.get("video_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return VideoResponse::default();
    }
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    }
}

/// Extracts the finished video from a DashScope (Qwen) poll response. Qwen
/// uses url delivery: the finished video sits at output.video_url, so
/// VideoData.url carries the temporary DashScope-hosted URL and bytes stays
/// empty.
fn video_result_from_qwen(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("output")
        .and_then(|o| o.get("video_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return VideoResponse::default();
    }
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    }
}

/// Performs the two-hop file-retrieve step for providers whose terminal poll
/// yields a file reference rather than a finished video URL (vg_cfg.file_endpoint
/// set, e.g. minimax): extracts the file id from the terminal poll body, GETs
/// the file endpoint (joined to the resolved video base), and extracts the
/// finished reference. file-id and result locations are wire-shape-keyed (the
/// transform); the endpoint is config.
async fn resolve_video_file(
    base: &str,
    vg_cfg: &VideoGenDef,
    poll_body: &str,
    headers: &[(String, String)],
) -> Result<VideoResponse, Error> {
    let poll: Value = serde_json::from_str(poll_body)?;
    let file_id = video_file_id(poll.get("file_id"));
    if file_id.is_empty() {
        return Err(Error::Unsupported(
            "video file hop: terminal poll carried no file_id".into(),
        ));
    }
    let url = format!(
        "{base}{}",
        vg_cfg.file_endpoint.replace("{file_id}", &file_id)
    );
    let (status, file_body) = get_text(&url, headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: "video_file_retrieve".into(),
            status_code: status.as_u16(),
            message: file_body,
        });
    }
    let file_raw: Value = serde_json::from_str(&file_body)?;
    Ok(video_result_from_minimax_file(vg_cfg, &file_raw))
}

/// Reads the minimax terminal poll's file_id, which the API may encode as a
/// string or a (large) integer.
fn video_file_id(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Extracts the finished video from a minimax file-retrieve response. minimax
/// uses url delivery: the download URL sits at file.download_url, so
/// VideoData.url carries it and bytes stays empty.
fn video_result_from_minimax_file(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("file")
        .and_then(|f| f.get("download_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return VideoResponse::default();
    }
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    }
}

/// Extracts the finished video reference from a Veo LRO poll response. Veo
/// uses download delivery: the response carries a temporary Files-API download
/// URI at response.generateVideoResponse.generatedSamples[0].video.uri. This
/// places it in VideoData.url; the wait_video download step
/// (output_delivery=DeliveryDownload) then fetches the bytes into
/// VideoData.bytes and clears url.
fn video_result_from_veo(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("response")
        .and_then(|r| r.get("generateVideoResponse"))
        .and_then(|g| g.get("generatedSamples"))
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
        .and_then(|first| first.get("video"))
        .and_then(|v| v.get("uri"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return VideoResponse::default();
    }
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    }
}

/// Extracts the finished video from a Vertex Veo fetchPredictOperation poll
/// response. Unlike Gemini Veo (which returns a fetch URI), Vertex Veo returns
/// the bytes inline as base64 at response.videos[0].bytesBase64Encoded with the
/// mime at .mimeType. This is download delivery with NO fetch hop: the bytes are
/// decoded straight into VideoData.bytes here and VideoData.url stays empty, so
/// the wait_video download step (download_video_bytes) finds no url and no-ops —
/// the source-XOR contract holds (VID-004: download delivery returns bytes,
/// never a url).
fn video_result_from_vertex_veo(vg_cfg: &VideoGenDef, raw: &Value) -> Result<VideoResponse, Error> {
    let mut mime = video_fallback_mime(vg_cfg);
    let first = raw
        .get("response")
        .and_then(|r| r.get("videos"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first());
    let first = match first {
        Some(f) => f,
        None => return Ok(VideoResponse::default()),
    };
    if let Some(m) = first
        .get("mimeType")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        mime = m.to_string();
    }
    let b64 = first
        .get("bytesBase64Encoded")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if b64.is_empty() {
        return Ok(VideoResponse::default());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| Error::Unsupported(format!("decode vertex video bytes: {e}")))?;
    Ok(VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url: String::new(),
            bytes,
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    })
}

/// Extracts the finished video reference from a Bedrock Nova Reel poll
/// response. Bedrock uses output-uri delivery: the provider wrote the mp4 to
/// the caller's own S3 bucket and the finished poll echoes the S3 URI at
/// outputDataConfig.s3OutputDataConfig.s3Uri. The SDK surfaces it as
/// VideoData.url with bytes empty — the wait_video delivery step never
/// downloads it (only DeliveryDownload fetches), so the caller fetches from S3
/// with their own tooling (VID-005; ADR-034 open question 4).
fn video_result_from_bedrock(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("outputDataConfig")
        .and_then(|o| o.get("s3OutputDataConfig"))
        .and_then(|s| s.get("s3Uri"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if url.is_empty() {
        return VideoResponse::default();
    }
    VideoResponse {
        videos: vec![VideoData {
            mime_type: mime,
            url,
            bytes: Vec::new(),
            duration_seconds: 0,
        }],
        ..VideoResponse::default()
    }
}

/// Reads the SigV4 credential env vars (region/secret/session) for a provider.
/// Empty when unset — mirrors Go's `os.Getenv`, so the keyless test path still
/// produces an `AWS4-HMAC-SHA256` Authorization header (the secret is never
/// verified by the mock).
fn sigv4_env(cfg: &ProviderSpec) -> (String, String, String) {
    let region = std::env::var(cfg.region_env_var).unwrap_or_default();
    let secret_key = std::env::var(cfg.secret_key_env_var).unwrap_or_default();
    let session_token = if cfg.session_token_env_var.is_empty() {
        String::new()
    } else {
        std::env::var(cfg.session_token_env_var).unwrap_or_default()
    };
    (region, secret_key, session_token)
}

/// Percent-encodes a Bedrock async-invoke ARN as a single path segment for the
/// GetAsyncInvoke poll: '/' → %2F (so the ARN does not split into extra path
/// segments) but ':' left literal (Bedrock's SigV4 canonicalization accepts a
/// literal ':' — the live Converse path signs a model id's ':' unescaped).
/// reqwest's `Url::path()` preserves the %2F, so the signed canonical path
/// equals the wire path.
fn path_escape_arn(arn: &str) -> String {
    arn.replace('/', "%2F")
}

/// Appends the provider's query-param API key to a video URL when the provider
/// authenticates that way (Google ?key=); a no-op for bearer-header providers
/// (every other video provider). Picks ? or & based on whether the URL already
/// carries a query string (the Files-API download URI arrives with ?alt=media).
fn append_video_auth(url: &str, provider: &Provider, cfg: &ProviderSpec) -> String {
    if !matches!(auth_scheme(provider.name), AuthScheme::QueryParamKey)
        || cfg.auth_query_param.is_empty()
    {
        return url.to_string();
    }
    let separator = if url.contains('?') { "&" } else { "?" };
    format!(
        "{url}{separator}{}={}",
        cfg.auth_query_param, provider.api_key
    )
}

/// Fetches the finished video for download-delivery providers
/// (vg_cfg.output_delivery == DeliveryDownload, e.g. Veo). The poll result
/// placed the temporary fetch URI in VideoData.url; this GETs each one
/// (carrying the provider's query-param auth when applicable) and moves the
/// payload into VideoData.bytes, clearing url so the source-XOR contract holds
/// (VID-004): download delivery returns bytes, never a url.
async fn download_video_bytes(
    provider: &Provider,
    cfg: &ProviderSpec,
    mut resp: VideoResponse,
) -> Result<VideoResponse, Error> {
    let headers = build_auth_headers(provider, cfg);
    for video in &mut resp.videos {
        if video.url.is_empty() {
            continue;
        }
        let fetch_url = append_video_auth(&video.url, provider, cfg);
        let (status, body) = get_bytes(&fetch_url, &headers).await?;
        if !status.is_success() {
            return Err(Error::Api {
                provider: "video_download".into(),
                status_code: status.as_u16(),
                message: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        video.bytes = body;
        video.url = String::new();
    }
    Ok(resp)
}

/// Returns the first model's output MIME, used when the provider does not
/// echo a MIME on the result.
fn video_fallback_mime(vg_cfg: &VideoGenDef) -> String {
    match vg_cfg.models.first() {
        Some(m) => m.output_mime.to_string(),
        None => "video/mp4".to_string(),
    }
}

/// Enforce the XOR rule and produce the canonical `Vec<Part>`. When only
/// `prompt` is set, synthesise `vec![Part::text(prompt)]`. Both empty or
/// both set returns Error::Validation.
fn normalize_video_parts(request: &VideoRequest) -> Result<Vec<Part>, Error> {
    let has_prompt = !request.prompt.is_empty();
    let has_parts = !request.parts.is_empty();
    match (has_prompt, has_parts) {
        (true, true) => Err(Error::Validation {
            field: "parts",
            message: "set prompt or parts, not both".into(),
        }),
        (false, false) => Err(Error::Validation {
            field: "prompt",
            message: "set either prompt or parts".into(),
        }),
        (true, false) => Ok(vec![Part::text(request.prompt.clone())]),
        (false, true) => Ok(request.parts.clone()),
    }
}

fn find_video_model<'a>(cfg: &'a VideoGenDef, model_id: &str) -> Option<&'a VideoModelDef> {
    cfg.models.iter().find(|m| m.model_id == model_id)
}

fn join_prompt_text(parts: &[Part]) -> String {
    let mut texts: Vec<&str> = Vec::new();
    for p in parts {
        if let Part::Text(s) = p {
            if !s.is_empty() {
                texts.push(s);
            }
        }
    }
    texts.join("\n")
}

// video_seed_image_url builds the image-to-video seed-frame data URL for wire
// shapes that condition on a single reference frame (Grok Imagine, BUG-010).
// The image Part's bytes are inlined as a data URL carried in xAI's image.url
// field, mirroring the Grok image-edit encoding in image.rs. Returns None when
// no image part is present (the text-to-video hot path). Errors on more than one
// image part: Grok animates a single seed frame, so multi-image conditioning is
// a separate slice — rejecting is honest where silently using the first would
// reintroduce the silent-drop bug.
fn video_seed_image_url(parts: &[Part]) -> Result<Option<String>, Error> {
    let mut seed: Option<&MediaRef> = None;
    for p in parts {
        if let Part::Image(media) = p {
            if seed.is_some() {
                return Err(Error::Validation {
                    field: "parts",
                    message: "image-to-video conditions on a single seed frame; pass one image part"
                        .into(),
                });
            }
            seed = Some(media);
        }
    }
    let Some(media) = seed else {
        return Ok(None);
    };
    let mime = if media.mime_type.is_empty() {
        "image/png"
    } else {
        &media.mime_type
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(&media.bytes);
    Ok(Some(format!("data:{mime};base64,{b64}")))
}
