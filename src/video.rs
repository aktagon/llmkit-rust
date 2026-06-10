//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use serde_json::{json, Value};
use std::time::Duration;

use crate::error::Error;
use crate::http::{get_text, post_json};
use crate::image::Part;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::providers::{provider_config, ProviderConfig};
use crate::providers::generated::video_gen::{video_gen_config, VideoGenDef, VideoModelDef};
use crate::request::{build_auth_headers, validate_provider};
use crate::structs::{VideoData, VideoHandle, VideoResponse};
use crate::types::Provider;

//
//
//
//
//
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(600);

///
///
///
///
///
///
///
///
///
///
///
///
#[derive(Clone, Debug, Default)]
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    pub parts: Vec<Part>,
}

///
///
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

///
///
///
///
///
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

///
///
///
///
///
///
///
///
///
///
///
///
///
async fn dispatch_video_submit(
    vg_cfg: &VideoGenDef,
    base: &str,
    headers: &[(String, String)],
    model: &str,
    parts: &[Part],
) -> Result<String, Error> {
    //
    //
    let (body, post_headers) = if vg_cfg.wire_shape == "VideoQwen" {
        //
        //
        let mut h = headers.to_vec();
        h.push(("X-DashScope-Async".to_string(), "enable".to_string()));
        (
            json!({
                "model": model,
                "input": { "prompt": join_prompt_text(parts) },
            }),
            h,
        )
    } else {
        (
            json!({
                "model": model,
                "prompt": join_prompt_text(parts),
            }),
            headers.to_vec(),
        )
    };
    let url = format!("{base}{}", vg_cfg.gen_endpoint);
    let (status, response_body) = post_json(&url, body, &post_headers).await?;
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

///
///
///
///
///
pub async fn wait_video(handle: &VideoHandle, poll: VideoPoll) -> Result<VideoResponse, Error> {
    let provider = &handle.provider;
    let cfg = provider_config(provider.name);
    let vg_cfg = video_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support video generation", provider.name),
    })?;

    let base = video_base_url(provider, cfg, vg_cfg);
    let headers = build_auth_headers(provider, cfg);
    let poll_url = video_poll_url(vg_cfg.poll_endpoint, &base, &handle.id);

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

///
///
///
///
///
fn video_base_url(provider: &Provider, cfg: &ProviderConfig, vg_cfg: &VideoGenDef) -> String {
    if let Some(b) = &provider.base_url {
        return b.clone();
    }
    if !vg_cfg.video_base_url.is_empty() {
        return vg_cfg.video_base_url.to_string();
    }
    cfg.base_url.to_string()
}

///
///
fn video_poll_url(poll_endpoint: &str, base: &str, id: &str) -> String {
    format!("{base}{}", poll_endpoint.replace("{id}", id))
}

///
///
///
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

///
///
///
///
///
///
///
///
///
///
///
///
///
fn parse_video_poll(vg_cfg: &VideoGenDef, body: &str) -> Result<(VideoResponse, bool), Error> {
    let raw: Value = serde_json::from_str(body)?;

    //
    //
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
                //
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
                //
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
                //
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
                //
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        other => Err(Error::Unsupported(format!(
            "video poll: unsupported wire shape {other:?}"
        ))),
    }
}

///
///
///
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

///
///
///
///
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

///
///
///
///
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

///
///
///
///
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

///
///
fn video_fallback_mime(vg_cfg: &VideoGenDef) -> String {
    match vg_cfg.models.first() {
        Some(m) => m.output_mime.to_string(),
        None => "video/mp4".to_string(),
    }
}

///
///
///
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
