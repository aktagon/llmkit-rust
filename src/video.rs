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

use base64::Engine;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::Error;
use crate::http::{get_bytes, get_text, get_text_sigv4, post_json, post_json_sigv4};
use crate::image::Part;
use crate::middleware::{fire_post, fire_pre, set_event_error, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::providers::generated::video_gen::{video_gen_config, VideoGenDef, VideoModelDef};
use crate::request::{build_auth_headers, validate_provider};
use crate::structs::{MediaRef, VideoData, VideoHandle, VideoResponse};
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
#
pub struct VideoRequest {
    pub model: String,
    pub prompt: String,
    pub parts: Vec<Part>,

    ///
    ///
    ///
    ///
    pub output_uri: String,
}

///
///
#
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
            Part::AudioUrl(_) | Part::AudioBytes(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "video generation does not accept audio parts".into(),
                });
            }
            Part::Image(_) => {
                //
                //
                //
                //
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
    //
    //
    //
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
        set_event_error(&mut post_event, err);
    }
    fire_post(middleware, &post_event);

    let request_id = result?;

    Ok(VideoHandle {
        id: request_id,
        provider: provider.clone(),
        raw,
        //
        //
        //
        model: request.model.clone(),
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
///
///
///
///
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
    } else if vg_cfg.wire_shape == "VideoPixVerse" {
        //
        //
        //
        //
        //
        let mut h = headers.to_vec();
        h.push(("Ai-trace-id".to_string(), new_video_trace_id()));
        (
            json!({
                "model": model,
                "prompt": join_prompt_text(parts),
                "duration": 5,
                "quality": "540p",
                "aspect_ratio": "16:9",
            }),
            h,
        )
    } else if vg_cfg.wire_shape == "VideoVeo" || vg_cfg.wire_shape == "VideoVertexVeo" {
        //
        //
        //
        //
        (
            json!({
                "instances": [{ "prompt": join_prompt_text(parts) }],
            }),
            headers.to_vec(),
        )
    } else if vg_cfg.wire_shape == "VideoBedrock" {
        //
        //
        //
        //
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
        //
        //
        //
        //
        //
        let mut b = json!({
            "model": model,
            "prompt": join_prompt_text(parts),
        });
        if let Some(seed) = video_seed_image_url(parts)? {
            b["image"] = json!({ "url": seed });
        }
        (b, headers.to_vec())
    };
    //
    //
    //
    let url = append_video_auth(
        &format!("{base}{}", vg_cfg.gen_endpoint.replace("{model}", model)),
        provider,
        cfg,
    );
    //
    //
    //
    let (status, response_body) = if matches!(auth_scheme(provider.name), AuthScheme::SigV4) {
        let (region, secret_key, session_token) = sigv4_env(cfg);
        //
        let caller_headers: Vec<(String, String)> = provider
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        post_json_sigv4(
            &url,
            body,
            &provider.api_key,
            &secret_key,
            &session_token,
            &region,
            cfg.service_name,
            &caller_headers,
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
    let mut headers = build_auth_headers(provider, cfg);
    //
    //
    //
    if vg_cfg.wire_shape == "VideoPixVerse" {
        headers.push(("Ai-trace-id".to_string(), new_video_trace_id()));
    }

    //
    //
    //
    //
    //
    //
    //
    //
    //
    //
    //
    //
    //
    //
    //
    let sigv4 = matches!(auth_scheme(provider.name), AuthScheme::SigV4);
    let vertex_poll = !sigv4 && vg_cfg.wire_shape == "VideoVertexVeo";
    let (poll_url, vertex_poll_body) = if sigv4 {
        //
        //
        //
        //
        //
        //
        //
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
        //
        //
        //
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
            //
            let caller_headers: Vec<(String, String)> = provider
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            get_text_sigv4(
                &poll_url,
                &provider.api_key,
                &secret_key,
                &session_token,
                &region,
                cfg.service_name,
                &caller_headers,
            )
            .await?
        } else if let Some(body) = &vertex_poll_body {
            //
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
            //
            //
            //
            let mut final_resp = if !vg_cfg.file_endpoint.is_empty() {
                resolve_video_file(&base, vg_cfg, &response_body, &headers).await?
            } else {
                resp
            };
            //
            //
            //
            //
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

///
///
///
///
///
fn video_base_url(provider: &Provider, cfg: &ProviderSpec, vg_cfg: &VideoGenDef) -> String {
    if let Some(b) = &provider.base_url {
        return b.clone();
    }
    let mut base = if !vg_cfg.video_base_url.is_empty() {
        vg_cfg.video_base_url.to_string()
    } else {
        cfg.base_url.to_string()
    };
    //
    //
    //
    if !cfg.region_env_var.is_empty() {
        let region = std::env::var(cfg.region_env_var).unwrap_or_default();
        base = base.replace("{region}", &region);
    }
    base
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
    //
    //
    //
    match cur {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
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
        "VideoVidu" => {
            //
            //
            //
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
                //
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoPixVerse" => {
            //
            //
            //
            //
            let status = raw
                .get("Resp")
                .and_then(|r| r.get("status"))
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);
            match status {
                1 => Ok((video_result_from_pixverse(vg_cfg, &raw), true)),
                7 | 8 => Err(Error::Unsupported(format!(
                    "video generation failed (status {status})"
                ))),
                //
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoMinimax" => {
            //
            //
            //
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "Success" => Ok((VideoResponse::default(), true)),
                "Fail" => Err(Error::Unsupported("video generation failed".into())),
                //
                _ => Ok((VideoResponse::default(), false)),
            }
        }
        "VideoVeo" => {
            //
            //
            //
            //
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
            //
            //
            //
            //
            let result = video_result_from_veo(vg_cfg, &raw);
            if result.videos.first().map(|v| v.url.is_empty()).unwrap_or(true) {
                return Err(Error::Unsupported(
                    "video generation: operation done but carried no video uri".into(),
                ));
            }
            Ok((result, true))
        }
        "VideoVertexVeo" => {
            //
            //
            //
            //
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
            //
            //
            if result.videos.first().map(|v| v.bytes.is_empty()).unwrap_or(true) {
                return Err(Error::Unsupported(
                    "video generation: operation done but carried no video bytes".into(),
                ));
            }
            Ok((result, true))
        }
        "VideoBedrock" => {
            //
            //
            //
            //
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "Completed" => {
                    //
                    //
                    //
                    //
                    //
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

///
///
///
///
fn video_result_from_pixverse(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("Resp")
        .and_then(|r| r.get("url"))
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
///
///
///
///
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

///
///
fn video_file_id(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

///
///
///
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

///
///
///
///
///
///
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

///
///
///
///
///
///
///
///
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

///
///
///
///
///
///
///
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

///
///
///
///
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

///
///
///
///
///
///
///
///
///
fn new_video_trace_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    //
    //
    //
    let hi = nanos ^ count.rotate_left(32);
    let lo = count ^ nanos.rotate_left(17);
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&hi.to_be_bytes());
    b[8..].copy_from_slice(&lo.to_be_bytes());
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant 0b10
    let h: String = b.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

///
///
///
///
///
///
fn path_escape_arn(arn: &str) -> String {
    arn.replace('/', "%2F")
}

///
///
///
///
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

///
///
///
///
///
///
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

//
//
//
//
//
//
//
//
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
