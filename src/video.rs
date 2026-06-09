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
//!
//! `submit_video` fires the `VideoGeneration` middleware op pre + post
//! around the HTTP submit (mirroring batch-submit semantics — never around
//! the wait poll loop). Mirrors music's `generate_music` fire pattern.

use serde_json::{json, Value};
use std::time::Duration;

use crate::error::Error;
use crate::http::{get_text, post_json};
use crate::image::Part;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::video_gen::{video_gen_config, VideoGenDef, VideoModelDef};
use crate::request::{build_auth_headers, validate_provider};
use crate::structs::{VideoData, VideoHandle, VideoResponse};
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
    for part in &parts {
        match part {
            Part::Lyrics(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "video generation does not accept lyrics parts".into(),
                });
            }
            Part::Image(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "image-to-video is not yet wired (slice 1 is text-to-video)".into(),
                });
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

    let vg_cfg = video_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support video generation", provider.name),
    })?;
    if find_video_model(vg_cfg, &request.model).is_none() {
        return Err(Error::Validation {
            field: "model",
            message: format!(
                "{} is not a known video-generation model for {:?}",
                request.model, provider.name
            ),
        });
    }

    let cfg = provider_config(provider.name);
    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| cfg.base_url.to_string());
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

    let result =
        dispatch_video_submit(vg_cfg, &base, &headers, &request.model, &parts).await;

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
    })
}

/// POSTs the submit body per wire shape (never by provider name) and
/// returns the provider-assigned poll handle id.
///
///   - VideoGrok (xAI) and VideoZhipu (CogVideoX) share the simple
///     {model, prompt} submit body. They differ only in which response field
///     carries the poll handle: Grok returns it as request_id, Zhipu as the
///     top-level id (alongside its own request_id, which is NOT the poll key).
///     A future shape with a different submit body adds an arm that builds
///     its own body.
async fn dispatch_video_submit(
    vg_cfg: &VideoGenDef,
    base: &str,
    headers: &[(String, String)],
    model: &str,
    parts: &[Part],
) -> Result<String, Error> {
    // The submit-response field holding the poll handle id is the only
    // per-shape difference for the {model, prompt} body. An unknown shape is
    // rejected (not defaulted to Grok) so a config-only provider addition that
    // forgets its runtime arm fails loud instead of silently POSTing as Grok.
    let id_field = match vg_cfg.wire_shape {
        "VideoGrok" => "request_id",
        "VideoZhipu" => "id",
        other => {
            return Err(Error::Unsupported(format!(
                "video submit: unsupported wire shape {other:?}"
            )))
        }
    };

    let body = json!({
        "model": model,
        "prompt": join_prompt_text(parts),
    });
    let url = format!("{base}{}", vg_cfg.gen_endpoint);
    let (status, response_body) = post_json(&url, body, headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: "video_submit".into(),
            status_code: status.as_u16(),
            message: response_body,
        });
    }
    let raw: Value = serde_json::from_str(&response_body)?;
    let id = raw
        .get(id_field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        return Err(Error::Unsupported(format!(
            "video submit: empty {id_field}"
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

    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| cfg.base_url.to_string());
    let headers = build_auth_headers(provider, cfg);
    let poll_url = video_poll_url(vg_cfg.wire_shape, &base, &handle.id);

    let deadline = std::time::Instant::now() + poll.timeout;
    loop {
        if std::time::Instant::now() > deadline {
            return Err(Error::Unsupported(format!(
                "video poll: timed out waiting for {}",
                handle.id
            )));
        }

        let (status, response_body) = get_text(&poll_url, &headers).await?;
        if !status.is_success() {
            return Err(Error::Api {
                provider: "video_poll".into(),
                status_code: status.as_u16(),
                message: response_body,
            });
        }

        let (mut resp, done) = parse_video_poll(vg_cfg, &response_body)?;
        if done {
            if handle.raw {
                resp.raw = serde_json::from_str(&response_body).ok();
            }
            return Ok(resp);
        }

        tokio::time::sleep(poll.interval).await;
    }
}

/// Builds the per-wire-shape poll URL.
///
///   - VideoGrok: GET {base}/v1/videos/{id}.
///   - VideoZhipu: GET {base}/v4/async-result/{id}.
fn video_poll_url(wire_shape: &str, base: &str, id: &str) -> String {
    match wire_shape {
        "VideoZhipu" => format!("{base}/v4/async-result/{id}"),
        _ => format!("{base}/v1/videos/{id}"), // VideoGrok
    }
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
fn parse_video_poll(vg_cfg: &VideoGenDef, body: &str) -> Result<(VideoResponse, bool), Error> {
    let raw: Value = serde_json::from_str(body)?;

    // Unknown shape rejected (not defaulted to Grok): a forgotten poll arm
    // fails loud instead of hanging on a never-terminal status.
    match vg_cfg.wire_shape {
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
