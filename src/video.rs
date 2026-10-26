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
//!   - VideoGrok: POST {model, prompt} to the variant's endpoint; submit response is
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::Error;
use crate::http::{get_bytes, get_text, get_text_sigv4, post_empty, post_json, post_json_sigv4};
use crate::image::Part;
use crate::middleware::{fire_post, fire_pre, set_event_error, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::image_gen::{
    WITH_LAST_FRAME, WITH_REFERENCE_IMAGES, WITH_SEED_FRAME,
};
use crate::providers::generated::providers::{provider_config, ProviderName, ProviderSpec};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::providers::generated::video_gen::{video_gen_config, VideoGenDef, VideoModelDef};
use crate::request::{build_auth_headers, validate_provider};
use crate::request_variant::{parts_carry, select_request_variant};
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

    /// The final frame the video ends on, sent with a first frame (an image
    /// part). Set it on the builder via `Video::last_frame` (BUG-089).
    pub last_frame: Option<MediaRef>,

    /// Asset images the video keeps, at most 3, never with a first or last
    /// frame. Set them on the builder via `Video::add_reference_image`.
    pub reference_images: Vec<MediaRef>,

    /// The video length in whole seconds; 0 means unset (the provider
    /// default). VideoOpenRouter and VideoGrok send it as the JSON integer
    /// `duration`; VideoVertexVeo sends it as the JSON number
    /// parameters.durationSeconds (HANDOFF-097, HANDOFF-100). Set it via
    /// `Video::duration`.
    pub duration_seconds: i64,

    /// The output aspect ratio, such as "16:9"; empty means unset. Sent as
    /// `aspect_ratio` or parameters.aspectRatio, on the same wire shapes as
    /// `duration_seconds`. Set it via `Video::aspect_ratio`.
    pub aspect_ratio: String,

    /// The output resolution, such as "720p"; empty means unset. Sent as
    /// `resolution` or parameters.resolution, on the same wire shapes as
    /// `duration_seconds`. Set it via `Video::resolution`.
    pub resolution: String,
}

/// The most asset reference images a Vertex Veo request takes (Vertex Veo
/// page, read 2026-10-15).
const MAX_VIDEO_REFERENCE_IMAGES: usize = 3;

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
            Part::AudioUrl(_) | Part::AudioBytes(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "video generation does not accept audio parts".into(),
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
    let image_count = parts.iter().filter(|p| p.is_image()).count();
    validate_video_frames(model, image_count, request)?;
    validate_video_options(provider.name, vg_cfg, model, request)?;
    // ADR-096: the carried inputs select the submit request; a config with
    // no such variant cannot send it.
    let variant = select_request_variant(
        provider.name,
        vg_cfg.variants,
        &video_carries(image_count, request),
    )?;
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
    // ADR-096: the selected variant's endpoint, joined to the video base.
    let submit_endpoint = format!("{base}{}", variant.endpoint);
    let mut headers = build_auth_headers(provider, cfg);
    headers.push(("content-type".into(), "application/json".into()));

    let base_event = Event {
        op: MiddlewareOp::VideoGeneration,
        provider: format!("{:?}", provider.name),
        model: request.model.clone(),
        ..Event::default()
    };
    let start = crate::clock::Instant::now();
    fire_pre(middleware, &base_event)?;

    let result = dispatch_video_submit(
        provider,
        cfg,
        vg_cfg,
        &submit_endpoint,
        &headers,
        request,
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
        // Carried so wait can build a model-templated poll URL (Vertex Veo
        // polls POST /{model}:fetchPredictOperation). Empty-ignored by every
        // other shape's poll endpoint.
        model: request.model.clone(),
    })
}

/// POSTs the submit body per wire shape (never by provider name) and
/// returns the provider-assigned poll handle id.
///
///   - VideoGrok (xAI), VideoZhipu (CogVideoX), VideoTogether, VideoMinimax and
///     VideoVidu share the simple {model, prompt} submit body. They differ
///     only in which response
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
    submit_endpoint: &str,
    headers: &[(String, String)],
    request: &VideoRequest,
    parts: &[Part],
) -> Result<String, Error> {
    let model = request.model.as_str();
    // Submit endpoint: the config-declared base + the selected request
    // variant's relative path (Option D, ADR-096), joined by the caller;
    // handle id from the config-declared dotted path (OQ7).
    let (body, post_headers) = build_video_submit_body(vg_cfg.wire_shape, headers, request, parts)?;
    // {model} in the submit endpoint is substituted with the per-call model
    // (Veo's :predictLongRunning path); a no-op for providers that carry the
    // model in the body. Query-param auth (Google ?key=) is appended last.
    let url = append_video_auth(&submit_endpoint.replace("{model}", model), provider, cfg);
    // Bedrock signs every request (SigV4); the bearer/query-param header map
    // does not apply. Region/secret/session come from the AWS env vars (empty
    // when unset — like Go's os.Getenv — so the keyless test path still signs).
    let (status, response_body) = if matches!(auth_scheme(provider.name), AuthScheme::SigV4) {
        let (region, secret_key, session_token) = sigv4_env(cfg);
        // ADR-052: caller custom headers ride alongside the signed request.
        let caller_headers: Vec<(String, String)> = provider
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        post_json_sigv4(
            &url,
            provider.timeout,
            body,
            &crate::http::SigV4Credentials {
                access_key: &provider.api_key,
                secret_key: &secret_key,
                session_token: &session_token,
                region: &region,
                service: cfg.service_name,
            },
            &caller_headers,
        )
        .await?
    } else {
        post_json(&url, provider.timeout, body, &post_headers).await?
    };
    if !status.is_success() {
        return Err(Error::Api {
            provider: "video_submit".into(),
            status_code: status.as_u16(),
            reason: crate::response::api_error_reason(provider.name, &response_body),
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

/// Selects the submit body and per-shape headers by wire shape (never by
/// provider name).
fn build_video_submit_body(
    wire_shape: &str,
    headers: &[(String, String)],
    request: &VideoRequest,
    parts: &[Part],
) -> Result<(Value, Vec<(String, String)>), Error> {
    let model = request.model.as_str();
    let output_uri = request.output_uri.as_str();
    Ok(if wire_shape == "VideoQwen" {
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
    } else if wire_shape == "VideoPixVerse" {
        // PixVerse requires all five fields; the generic surface is prompt-only,
        // so duration/quality/aspect_ratio are sent as reference-anchored
        // defaults (valid across the recorded models). The per-request
        // Ai-trace-id header (UUID, unique per request) is PixVerse's anti-cache
        // key; set per-request only so it never leaks into the shared header map.
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
    } else if wire_shape == "VideoVertexVeo" {
        // Vertex Veo carries the model in the submit PATH (:predictLongRunning), not the body — so the body has
        // no model field. The prompt nests under instances[]; the optional
        // parameters object is omitted on the prompt-only hot path.
        let mut instance = json!({ "prompt": join_prompt_text(parts) });
        // First frame (BUG-089): the seed rides at instances[0].image as raw
        // base64 + mimeType (no data: prefix), per the Vertex Veo reference
        // (read 2026-10-15).
        if let Some(seed) = video_seed_image(parts)? {
            instance["image"] = veo_image(seed);
        }
        // Last frame and asset reference images (BUG-089): same
        // {bytesBase64Encoded, mimeType} wrapper. Reference images force an
        // 8-second video, sent as the NUMBER 8.
        if let Some(last) = &request.last_frame {
            instance["lastFrame"] = veo_image(last);
        }
        let mut body = json!({ "instances": [] });
        let mut params = serde_json::Map::new();
        if !request.reference_images.is_empty() {
            instance["referenceImages"] = request
                .reference_images
                .iter()
                .map(|img| json!({ "image": veo_image(img), "referenceType": "asset" }))
                .collect();
            params.insert("durationSeconds".into(), json!(8));
        }
        // Video options (HANDOFF-100): parameters.durationSeconds (a JSON
        // number), aspectRatio and resolution, each only when set. With
        // reference images validate_video_options allows only 8, so a set
        // duration matches the forced 8.
        if request.duration_seconds > 0 {
            params.insert("durationSeconds".into(), json!(request.duration_seconds));
        }
        if !request.aspect_ratio.is_empty() {
            params.insert("aspectRatio".into(), json!(request.aspect_ratio));
        }
        if !request.resolution.is_empty() {
            params.insert("resolution".into(), json!(request.resolution));
        }
        if !params.is_empty() {
            body["parameters"] = Value::Object(params);
        }
        body["instances"] = json!([instance]);
        (body, headers.to_vec())
    } else if wire_shape == "VideoBedrock" {
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
    } else if wire_shape == "VideoOpenRouter" {
        // Image-to-video (HANDOFF-091): each frame is an OpenRouter
        // ContentPartImage plus frame_type, first_frame then last_frame (live
        // probe 2026-10-18; openrouter.ai/openapi.json).
        let mut b = json!({
            "model": model,
            "prompt": join_prompt_text(parts),
        });
        if let Some(seed) = video_seed_image(parts)? {
            let mut frames = vec![openrouter_frame(seed, "first_frame")];
            if let Some(last) = &request.last_frame {
                frames.push(openrouter_frame(last, "last_frame"));
            }
            b["frame_images"] = Value::Array(frames);
        }
        set_root_video_options(&mut b, request);
        (b, headers.to_vec())
    } else if matches!(
        wire_shape,
        "VideoGrok" | "VideoZhipu" | "VideoTogether" | "VideoMinimax" | "VideoVidu"
    ) {
        // Image-to-video (BUG-010): when a seed frame is present (reachable in
        // this arm only for grok-imagine-video; Veo has its own arm above),
        // inline it as a data URL in xAI's image.url field — the same
        // encoding the Grok image-edit path uses. Absent on the text-to-video
        // hot path, so the existing video-grok golden is unchanged.
        let mut b = json!({
            "model": model,
            "prompt": join_prompt_text(parts),
        });
        if let Some(seed) = video_seed_image_url(parts)? {
            b["image"] = json!({ "url": seed });
        }
        // Only VideoGrok in this arm sends the options (HANDOFF-100); the
        // other shapes reject a set option in validate_video_options.
        if wire_shape == "VideoGrok" {
            set_root_video_options(&mut b, request);
        }
        (b, headers.to_vec())
    } else {
        return Err(Error::Unsupported(format!(
            "video submit: unsupported wire shape {wire_shape:?}"
        )));
    })
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
    let mut headers = build_auth_headers(provider, cfg);
    // PixVerse requires the per-request Ai-trace-id header on the poll GET too
    // (not just submit); one trace id per wait call is sufficient — uniqueness
    // is an anti-cache measure on the generation, and the poll is a read.
    if vg_cfg.wire_shape == "VideoPixVerse" {
        headers.push(("Ai-trace-id".to_string(), new_video_trace_id()));
    }

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
    //
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
            &format!(
                "{base}{}",
                vg_cfg.poll_endpoint.replace("{model}", &handle.model)
            ),
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

    let deadline = crate::clock::Instant::now() + poll.timeout;
    loop {
        if crate::clock::Instant::now() > deadline {
            // ADR-063 POLL-008: the same typed backstop job.rs returns.
            return Err(Error::PollTimeout {
                provider: format!("{:?}", provider.name),
                id: handle.id.clone(),
            });
        }

        let (status, response_body) = if sigv4 {
            let (region, secret_key, session_token) = sigv4_env(cfg);
            // ADR-052: caller custom headers ride alongside the signed poll.
            let caller_headers: Vec<(String, String)> = provider
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            get_text_sigv4(
                &poll_url,
                provider.timeout,
                &crate::http::SigV4Credentials {
                    access_key: &provider.api_key,
                    secret_key: &secret_key,
                    session_token: &session_token,
                    region: &region,
                    service: cfg.service_name,
                },
                &caller_headers,
            )
            .await?
        } else if let Some(body) = &vertex_poll_body {
            // Vertex Veo is the only POST-poll shape (fetchPredictOperation).
            post_json(&poll_url, provider.timeout, body.clone(), &headers).await?
        } else {
            get_text(&poll_url, provider.timeout, &headers).await?
        };
        if !status.is_success() {
            return Err(Error::Api {
                provider: "video_poll".into(),
                status_code: status.as_u16(),
                reason: crate::response::api_error_reason(provider.name, &response_body),
                message: response_body,
            });
        }

        let resp = match parse_video_poll(vg_cfg, &response_body)? {
            VideoPollState::Running => None,
            VideoPollState::Done(resp) => Some(resp),
            VideoPollState::Failed { status, message } => {
                return Err(Error::JobFailed {
                    job: VIDEO_JOB,
                    provider: format!("{:?}", provider.name),
                    id: handle.id.clone(),
                    status,
                    message,
                });
            }
        };
        if let Some(resp) = resp {
            // Two-hop providers (vg_cfg.file_endpoint set, e.g. minimax): the
            // terminal poll carried a file reference, not a video URL — resolve
            // it with one more GET before returning.
            let mut final_resp = if !vg_cfg.file_endpoint.is_empty() {
                resolve_video_file(&base, provider.timeout, vg_cfg, &response_body, &headers)
                    .await?
            } else {
                resp
            };
            // Delivery dispatch (VID-005). Download-delivery providers
            // (OpenRouter) returned a temporary fetch URI in VideoData.url; GET it and fill
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

        crate::clock::sleep(poll.interval).await?;
    }
}

/// Asks the provider to cancel a running video job, given the handle
/// [`submit_video`] returned. Sends one request and returns when the provider
/// accepts it; it does not poll for the outcome.
///
/// Google calls a cancel best effort: success is not guaranteed, and the job
/// may still finish. A cancelled operation ends with error code 1
/// (CANCELLED). No source says a cancel stops billing or lowers the cost.
///
/// The cancel endpoint is provider data. Vertex Veo sends
/// `POST {base}/{model}/operations/{operation_id}:cancel` with an empty body,
/// which is Google's general long-running-operation cancel
/// (`/v1/{name}:cancel`). That Vertex accepts it for a Veo operation is
/// unconfirmed: no Google page says so.
///
/// Errors:
/// - [`Error::Unsupported`] when the provider documents no cancel for video
///   jobs. Nothing is sent.
/// - [`Error::Validation`] when the provider has no video generation, or the
///   handle id is empty.
/// - [`Error::Api`] when the provider answers with a non-2xx status.
pub async fn cancel_video(handle: &VideoHandle) -> Result<(), Error> {
    let provider = &handle.provider;
    let cfg = provider_config(provider.name);
    let vg_cfg = video_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support video generation", provider.name),
    })?;
    if vg_cfg.cancel_endpoint.is_empty() {
        return Err(Error::Unsupported(format!(
            "video cancel: {:?} has no cancel endpoint for video jobs",
            provider.name
        )));
    }
    // The operation id is the last segment of the operation name the submit
    // returned (projects/../models/{model}/operations/{operation_id}).
    let operation_id = handle.id.rsplit('/').next().unwrap_or_default();
    if operation_id.is_empty() {
        return Err(Error::Validation {
            field: "id",
            message: "video cancel needs the handle id submit returned".into(),
        });
    }

    let base = video_base_url(provider, cfg, vg_cfg);
    let path = vg_cfg
        .cancel_endpoint
        .replace("{model}", &handle.model)
        .replace("{operation_id}", operation_id);
    let url = append_video_auth(&format!("{base}{path}"), provider, cfg);
    let headers = build_auth_headers(provider, cfg);
    let (status, response_body) = post_empty(&url, provider.timeout, &headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: "video_cancel".into(),
            status_code: status.as_u16(),
            reason: crate::response::api_error_reason(provider.name, &response_body),
            message: response_body,
        });
    }
    Ok(())
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

///
/// it to the resolved video base.
fn video_poll_url(poll_endpoint: &str, base: &str, id: &str) -> String {
    format!("{base}{}", poll_endpoint.replace("{id}", id))
}

/// Descends a dotted path (e.g. "id", "output.task_id") through the decoded
/// submit response, returning the leaf or "" if a segment is missing or the
/// leaf is neither a string nor a number.
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
    // The leaf is usually a string handle, but some providers return a numeric
    // job id (PixVerse's Resp.video_id is an integer) — format it back to its
    // integer string form.
    match cur {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// What `Error::JobFailed::job` says for a video job.
const VIDEO_JOB: &str = "video generation";

/// The message of a failed Vidu, Vertex Veo or Bedrock job whose reply
/// carries no message. These shapes used this text before BUG-103, as Go and
/// Java's Vidu arm still do.
const NO_MESSAGE: &str = "operation failed";

/// One classified video poll (BUG-103): still running, finished, or ended by
/// the provider with a failure. `wait_video` turns `Failed` into
/// `Error::JobFailed`, adding the provider and job id the parser lacks.
enum VideoPollState {
    Running,
    Done(VideoResponse),
    /// The provider's raw terminal status and its error message. A shape
    /// that sends only a status has an empty message; a shape with a
    /// message field that sent none has [`NO_MESSAGE`].
    Failed {
        status: String,
        message: String,
    },
}

fn failed(status: &str, message: &str) -> VideoPollState {
    VideoPollState::Failed {
        status: status.to_string(),
        message: message.to_string(),
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
///   - VideoTogether: {"status": "completed"|"failed"|"cancelled"|"queued"|
///     "in_progress", "outputs": {"video_url"}}.
///   - VideoQwen: {"output": {"task_status": "SUCCEEDED"|"FAILED"|"CANCELED"|
///     "PENDING"|"RUNNING"|"UNKNOWN", "video_url"}}.
///   - VideoOpenRouter: {"status": "completed"|"failed"|"cancelled"|"expired"|
///     "pending"|"in_progress", "unsigned_urls": [url], "error": "..."}.
fn parse_video_poll(vg_cfg: &VideoGenDef, body: &str) -> Result<VideoPollState, Error> {
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
                "SUCCEEDED" => Ok(VideoPollState::Done(video_result_from_qwen(vg_cfg, &raw))),
                "FAILED" | "CANCELED" => Ok(failed(status, "")),
                // PENDING, RUNNING, UNKNOWN (or any non-terminal status)
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoTogether" => {
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "completed" => Ok(VideoPollState::Done(video_result_from_together(
                    vg_cfg, &raw,
                ))),
                "failed" | "cancelled" => Ok(failed(status, "")),
                // queued, in_progress (or any non-terminal status)
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoOpenRouter" => {
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "completed" => {
                    // A completed poll without unsigned_urls[0] must be an
                    // error, not an empty success (mirrors the Vertex Veo
                    // done-without-bytes guard).
                    let result = video_result_from_openrouter(vg_cfg, &raw);
                    if result
                        .videos
                        .first()
                        .map(|v| v.url.is_empty())
                        .unwrap_or(true)
                    {
                        return Err(Error::Decoding(
                            "video generation: completed but carried no unsigned_urls".into(),
                        ));
                    }
                    Ok(VideoPollState::Done(result))
                }
                "failed" | "cancelled" | "expired" => {
                    let msg = raw
                        .get("error")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(NO_MESSAGE);
                    Ok(failed(status, msg))
                }
                // pending, in_progress (or any non-terminal status)
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoZhipu" => {
            let status = raw
                .get("task_status")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match status {
                "SUCCESS" => Ok(VideoPollState::Done(video_result_from_zhipu(vg_cfg, &raw))),
                "FAIL" => Ok(failed(status, "")),
                // PROCESSING (or any non-terminal status)
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoVidu" => {
            // Vidu (Shengshu) task poll: state success terminal-success, failed
            // terminal-error, created/queueing/processing pending. The finished
            // video URL sits at creations[0].url (url delivery, single-hop).
            let state = raw.get("state").and_then(|v| v.as_str()).unwrap_or("");
            match state {
                "success" => Ok(VideoPollState::Done(video_result_from_vidu(vg_cfg, &raw))),
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
                        .unwrap_or(NO_MESSAGE);
                    Ok(failed(state, msg))
                }
                // created, queueing, processing (or any non-terminal state)
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoPixVerse" => {
            // PixVerse status poll: the status is an INTEGER code nested under
            // Resp. 1=success (terminal), 7/8=failed (terminal-error),
            // 5=generating (pending). The finished video URL sits at Resp.url
            // (url delivery).
            let status = raw
                .get("Resp")
                .and_then(|r| r.get("status"))
                .and_then(|v| v.as_i64())
                .unwrap_or(-1);
            match status {
                1 => Ok(VideoPollState::Done(video_result_from_pixverse(
                    vg_cfg, &raw,
                ))),
                7 | 8 => Ok(failed(&status.to_string(), "")),
                // 5 (generating) or any non-terminal status
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoMinimax" => {
            // Two-hop: terminal-success yields a file_id, not a URL. Report
            // done with an empty result; wait_video performs the file-retrieve
            // hop (gated on vg_cfg.file_endpoint) and fills the URL.
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "Success" => Ok(VideoPollState::Done(VideoResponse::default())),
                "Fail" => Ok(failed(status, "")),
                // Queueing, Preparing, Processing (or any non-terminal status)
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoVertexVeo" => {
            // Vertex Veo operation poll (fetchPredictOperation), an LRO: poll
            // until done==true; a done op carrying an error object is a
            // terminal failure. The finished video arrives as inline base64 in
            // the poll body (response.videos[0].bytesBase64Encoded), not a
            // fetch URI.
            let done = raw.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
            if !done {
                return Ok(VideoPollState::Running);
            }
            if let Some(err_obj) = raw.get("error").filter(|e| e.is_object()) {
                let msg = err_obj
                    .get("message")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(NO_MESSAGE);
                return Ok(failed("error", msg));
            }
            let result = video_result_from_vertex_veo(vg_cfg, &raw)?;
            // A done op carrying no decodable bytes must surface as an error, not a silent zero-byte success.
            if result
                .videos
                .first()
                .map(|v| v.bytes.is_empty())
                .unwrap_or(true)
            {
                return Err(Error::Decoding(
                    "video generation: operation done but carried no video bytes".into(),
                ));
            }
            Ok(VideoPollState::Done(result))
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
                    // the Vertex Veo done-without-bytes guard): the caller would otherwise get
                    // a "successful" VideoResponse whose URL is empty and never
                    // find the mp4.
                    let result = video_result_from_bedrock(vg_cfg, &raw);
                    if result
                        .videos
                        .first()
                        .map(|v| v.url.is_empty())
                        .unwrap_or(true)
                    {
                        return Err(Error::Decoding(
                            "video generation: completed but carried no output s3 uri".into(),
                        ));
                    }
                    Ok(VideoPollState::Done(result))
                }
                "Failed" => {
                    let msg = raw
                        .get("failureMessage")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(NO_MESSAGE);
                    Ok(failed(status, msg))
                }
                // InProgress (or any non-terminal status)
                _ => Ok(VideoPollState::Running),
            }
        }
        "VideoGrok" => {
            let status = raw.get("status").and_then(|v| v.as_str()).unwrap_or("");
            match status {
                "done" => Ok(VideoPollState::Done(video_result_from_grok(vg_cfg, &raw))),
                "failed" | "expired" => {
                    let msg = raw
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    Ok(failed(status, msg))
                }
                // pending (or any non-terminal status)
                _ => Ok(VideoPollState::Running),
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
    let duration_seconds = video.get("duration").and_then(|v| v.as_i64()).unwrap_or(0);
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

/// Extracts the finished video from a PixVerse poll response. PixVerse uses
/// url delivery: the finished video sits at Resp.url (nested under the Resp
/// envelope), so VideoData.url carries the temporary PixVerse-hosted URL and
/// bytes stays empty.
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

/// Extracts the finished video from an OpenRouter poll response. OpenRouter
/// uses download delivery: unsigned_urls[0] is an absolute content URL that
/// needs the API key, so it goes into VideoData.url for the download step. The
/// signed urls[] array is a secret and is never read.
fn video_result_from_openrouter(vg_cfg: &VideoGenDef, raw: &Value) -> VideoResponse {
    let mime = video_fallback_mime(vg_cfg);
    let url = raw
        .get("unsigned_urls")
        .and_then(|u| u.get(0))
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
    timeout: std::time::Duration,
    vg_cfg: &VideoGenDef,
    poll_body: &str,
    headers: &[(String, String)],
) -> Result<VideoResponse, Error> {
    let poll: Value = serde_json::from_str(poll_body)?;
    let file_id = video_file_id(poll.get("file_id"));
    if file_id.is_empty() {
        return Err(Error::Decoding(
            "video file hop: terminal poll carried no file_id".into(),
        ));
    }
    let url = format!(
        "{base}{}",
        vg_cfg.file_endpoint.replace("{file_id}", &file_id)
    );
    let (status, file_body) = get_text(&url, timeout, headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: "video_file_retrieve".into(),
            status_code: status.as_u16(),
            message: file_body,
            reason: None,
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

/// Extracts the finished video from a Vertex Veo fetchPredictOperation poll
/// response. Vertex Veo returns the bytes inline as base64 at response.videos[0].bytesBase64Encoded with the
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
        .map_err(|e| Error::Decoding(format!("decode vertex video bytes: {e}")))?;
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

/// Returns a UUID-shaped (8-4-4-4-12 hex), unique-per-request trace id for
/// providers that require one (PixVerse's `Ai-trace-id`). This is a stdlib
/// unique-per-request trace id, NOT a CSPRNG UUID: the zero-dependency crate
/// (only `base64` allowed) cannot pull in `uuid`/`rand`/`getrandom`, and
/// PixVerse only needs uniqueness as an anti-cache key, not unpredictability.
/// Uniqueness is sourced from the nanosecond clock combined with a
/// process-global atomic counter (incremented per call), spread across 16
/// bytes; the version nibble is set to 4 and the variant bits to 0b10 so the
/// string is shaped like an RFC-4122 v4 UUID.
fn new_video_trace_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = crate::clock::unix_time().as_nanos() as u64;
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Two 64-bit lanes spread the clock + counter across 16 bytes; mixing the
    // counter into both lanes keeps successive calls within the same nanosecond
    // distinct.
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
/// (vg_cfg.output_delivery == DeliveryDownload, e.g. OpenRouter). The poll result
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
        let (status, body) = get_bytes(&fetch_url, provider.timeout, &headers).await?;
        if !status.is_success() {
            return Err(Error::Api {
                provider: "video_download".into(),
                status_code: status.as_u16(),
                message: String::from_utf8_lossy(&body).into_owned(),
                reason: crate::response::api_error_reason(
                    provider.name,
                    &String::from_utf8_lossy(&body),
                ),
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

// video_seed_image returns the single image-to-video seed frame, or None on the
// text-to-video hot path. Errors on more than one image part: every wired
// image-to-video shape (Grok, BUG-010; Vertex Veo first frame, BUG-089) conditions on
// a single seed frame, so rejecting is honest where silently using the first
// would reintroduce the silent-drop bug.
fn video_seed_image(parts: &[Part]) -> Result<Option<&MediaRef>, Error> {
    let mut seed: Option<&MediaRef> = None;
    for p in parts {
        if let Part::Image(media) = p {
            if seed.is_some() {
                return Err(Error::Validation {
                    field: "parts",
                    message:
                        "image-to-video conditions on a single seed frame; pass one image part"
                            .into(),
                });
            }
            seed = Some(media);
        }
    }
    Ok(seed)
}

// openrouter_frame is one OpenRouter frame_images item.
fn openrouter_frame(img: &MediaRef, frame_type: &str) -> Value {
    let b64 = base64::engine::general_purpose::STANDARD.encode(&img.bytes);
    json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{};base64,{b64}", video_seed_mime(img)) },
        "frame_type": frame_type,
    })
}

// veo_image is a Veo image object: raw base64 bytes and the MIME type.
fn veo_image(img: &MediaRef) -> Value {
    json!({
        "bytesBase64Encoded": base64::engine::general_purpose::STANDARD.encode(&img.bytes),
        "mimeType": video_seed_mime(img),
    })
}

// validate_video_frames rejects a last frame or reference images the model or
// the request cannot carry, before any HTTP call (BUG-089). Same rules, order
// and messages as go/video.go validateVideoFrames.
fn validate_video_frames(
    model: &VideoModelDef,
    image_count: usize,
    request: &VideoRequest,
) -> Result<(), Error> {
    let refs = request.reference_images.len();
    let last = request.last_frame.is_some();
    let (field, message) = if last && !model.supports_last_frame {
        (
            "last_frame",
            format!("{} does not accept a last frame", model.model_id),
        )
    } else if refs > 0 && !model.supports_reference_images {
        (
            "reference_images",
            format!("{} does not accept reference images", model.model_id),
        )
    } else if refs > MAX_VIDEO_REFERENCE_IMAGES {
        (
            "reference_images",
            format!("at most {MAX_VIDEO_REFERENCE_IMAGES} reference images, got {refs}"),
        )
    } else if refs > 0 && (image_count > 0 || last) {
        // Vertex documents the inputs as exclusive (live 2026-10-15, the Gemini
        // API Veo answered 400 "Unsupported video generation request").
        (
            "reference_images",
            "cannot be combined with a first or last frame".to_string(),
        )
    } else if last && image_count == 0 {
        ("last_frame", "requires a first frame".to_string())
    } else {
        return Ok(());
    };
    Err(Error::Validation { field, message })
}

// set_root_video_options sets the body-root duration, aspect_ratio and
// resolution of the VideoOpenRouter and VideoGrok bodies (HANDOFF-097,
// HANDOFF-100), each only when set, so the prompt-only body stays
// {model, prompt}. duration is a JSON integer; validate_video_options has
// already checked each value against the model.
fn set_root_video_options(body: &mut Value, request: &VideoRequest) {
    if request.duration_seconds > 0 {
        body["duration"] = json!(request.duration_seconds);
    }
    if !request.aspect_ratio.is_empty() {
        body["aspect_ratio"] = json!(request.aspect_ratio);
    }
    if !request.resolution.is_empty() {
        body["resolution"] = json!(request.resolution);
    }
}

// video_shape_sends_options reports whether a video wire shape has body fields
// for duration, aspect ratio and resolution (HANDOFF-097, HANDOFF-100).
fn video_shape_sends_options(shape: &str) -> bool {
    matches!(shape, "VideoOpenRouter" | "VideoGrok" | "VideoVertexVeo")
}

// validate_video_options rejects a duration, aspect ratio or resolution the
// provider or the model cannot take, before any HTTP call (HANDOFF-097). Same
// rules, order and messages as go/video.go validateVideoOptions:
//
//  1. A set option on a wire shape with no body field for it (see
//     video_shape_sends_options), naming the first set one in the order
//     duration, aspect_ratio, resolution.
//  2. A negative duration.
//  3. A duration above the model's max_duration_seconds (0 = no known limit).
//  4. A duration other than 8 with reference images: Veo forces 8 seconds
//     (ai.google.dev/gemini-api/docs/veo, read 2026-10-19; HANDOFF-100).
//  5. A resolution the model does not list (empty list = no fixed set).
//  6. An aspect ratio the model does not list (empty list = no fixed set).
fn validate_video_options(
    provider: ProviderName,
    vg_cfg: &VideoGenDef,
    model: &VideoModelDef,
    request: &VideoRequest,
) -> Result<(), Error> {
    if !video_shape_sends_options(vg_cfg.wire_shape) {
        let set = [
            ("duration", request.duration_seconds != 0),
            ("aspect_ratio", !request.aspect_ratio.is_empty()),
            ("resolution", !request.resolution.is_empty()),
        ];
        return match set.iter().find(|(_, is_set)| *is_set) {
            Some((field, _)) => Err(Error::Validation {
                field,
                message: format!("{provider} video does not send {field}"),
            }),
            None => Ok(()),
        };
    }
    let (d, max) = (request.duration_seconds, model.max_duration_seconds);
    let (r, a) = (request.resolution.as_str(), request.aspect_ratio.as_str());
    let (field, message) = if d < 0 {
        ("duration", "must be positive".to_string())
    } else if d > 0 && max > 0 && d > max {
        (
            "duration",
            format!("{} takes at most {max} seconds, got {d}", model.model_id),
        )
    } else if d > 0 && !request.reference_images.is_empty() && d != 8 {
        (
            "duration",
            format!("reference images take 8 seconds, got {d}"),
        )
    } else if !r.is_empty() && !model.resolutions.is_empty() && !model.resolutions.contains(&r) {
        (
            "resolution",
            format!("{} does not support resolution {r}", model.model_id),
        )
    } else if !a.is_empty() && !model.aspect_ratios.is_empty() && !model.aspect_ratios.contains(&a)
    {
        (
            "aspect_ratio",
            format!("{} does not support aspect ratio {a}", model.model_id),
        )
    } else {
        return Ok(());
    };
    Err(Error::Validation { field, message })
}

// video_carries is the request's carried inputs (ADR-096): reference images, a
// seed frame with an optional last frame, or text only.
fn video_carries(image_count: usize, request: &VideoRequest) -> Vec<&'static str> {
    if !request.reference_images.is_empty() {
        vec![WITH_REFERENCE_IMAGES]
    } else if image_count > 0 && request.last_frame.is_some() {
        vec![WITH_SEED_FRAME, WITH_LAST_FRAME]
    } else {
        parts_carry(image_count, WITH_SEED_FRAME)
    }
}

// video_seed_mime is the seed frame's MIME, defaulting to image/png.
fn video_seed_mime(media: &MediaRef) -> &str {
    if media.mime_type.is_empty() {
        "image/png"
    } else {
        &media.mime_type
    }
}

// video_seed_image_url builds the Grok image-to-video seed-frame data URL
// (BUG-010): the image Part's bytes inlined as a data URL carried in xAI's
// image.url field, mirroring the Grok image-edit encoding in image.rs. Returns
// None when no image part is present (the text-to-video hot path).
fn video_seed_image_url(parts: &[Part]) -> Result<Option<String>, Error> {
    let Some(media) = video_seed_image(parts)? else {
        return Ok(None);
    };
    let mime = video_seed_mime(media);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&media.bytes);
    Ok(Some(format!("data:{mime};base64,{b64}")))
}



























