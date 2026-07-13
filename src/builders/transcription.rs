//! Transcription (speech-to-text) runtime (ADR-048) — mirror of
//! go/transcription.go and go/transcription_builder.go.
//!
//! Wires Transcription::submit against `transcription_submit` and adds
//! `TranscriptionHandle::wait` via the `TranscriptionHandleExt` extension
//! trait (mirroring `VideoHandleExt` in builders/video.rs). Unlike video, the
//! whole runtime lives here (the slice has a single wire shape).
//!
//! Asynchronous: `transcription_submit` performs an optional upload hop for
//! local-bytes audio, POSTs the {audio_url} submit body, and returns a
//! [`TranscriptionHandle`] immediately; poll it with `wait`. Pre-flight
//! validation rejects an input that is not exactly one audio Part before any
//! HTTP call (STT-003). The submit/poll/status facts are config; only the
//! result decode is wire-shape-keyed (STT-005). Slice 1 wires
//! TranscriptionAssemblyAI: upload -> submit -> poll -> {text, words[]}.

use serde_json::{json, Value};
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::time::Duration;

use crate::error::Error;
use crate::http::{get_text, post_json, post_multipart};
use crate::image::Part;
use crate::job::{
    classify_by_config, non_empty_values, poll_job, poll_once, Classification, JobAdapter,
    JobStatus, LifecycleConfig, PollBody,
};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::transcription_gen::{transcription_config, TranscriptionDef};
use crate::request::{build_auth_headers, validate_provider};
use crate::structs::{TranscriptionHandle, TranscriptionResponse, TranscriptSegment};
use crate::types::Provider;

use super::Transcription;

// Default poll cadence for TranscriptionHandle::wait. AssemblyAI jobs run from
// seconds to minutes; the runtime polls every interval until timeout elapses.
// Mirror of go/transcription.go transcriptionPollInterval / Timeout.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(3);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(600);

/// Poll cadence for [`TranscriptionHandle::wait`]. Defaults match Go (3s
/// interval, 10min timeout); tests override `interval` to run fast.
#[derive(Clone, Copy, Debug)]
pub struct TranscriptionPoll {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for TranscriptionPoll {
    fn default() -> Self {
        Self {
            interval: DEFAULT_POLL_INTERVAL,
            timeout: DEFAULT_POLL_TIMEOUT,
        }
    }
}

pub(crate) async fn transcription_submit(
    b: Transcription,
    audio_parts: Vec<Part>,
) -> Result<TranscriptionHandle, Error> {
    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
    };
    submit_transcription(&provider, audio_parts).await
}

/// Submits an asynchronous speech-to-text job and returns a
/// [`TranscriptionHandle`] immediately. Poll the handle with `wait`. Pre-flight
/// validation rejects an input that is not exactly one audio Part before any
/// HTTP call (STT-003). For an audio-bytes part the runtime performs the upload
/// hop (POST the raw bytes, read upload_url) before submitting (STT-005).
pub async fn submit_transcription(
    provider: &Provider,
    parts: Vec<Part>,
) -> Result<TranscriptionHandle, Error> {
    validate_provider(provider)?;

    let tc_cfg = transcription_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support transcription", provider.name),
    })?;
    // A synchronous provider has no job handle; Submit/Wait is the wrong
    // terminal for it (ADR-051 OAA-003). Name the supported one.
    if tc_cfg.interaction == "sync" {
        return Err(Error::Validation {
            field: "interaction",
            message: format!(
                "{:?} transcribes synchronously; use Transcribe, not Submit/Wait",
                provider.name
            ),
        });
    }

    let (url, bytes) = normalize_audio_part(&parts)?;

    let cfg = provider_config(provider.name);
    let base = transcription_base_url(provider, cfg);
    let headers = build_auth_headers(provider, cfg);

    // Upload hop (STT-005): a bytes part is uploaded first to obtain a URL the
    // submit body can reference. URL parts skip this entirely.
    let audio_url = if let Some(raw) = bytes {
        if tc_cfg.upload_endpoint.is_empty() {
            return Err(Error::Validation {
                field: "parts",
                message: format!(
                    "{:?} does not accept audio bytes; pass a public audio URL",
                    provider.name
                ),
            });
        }
        let (status, body) =
            post_octet_stream(&format!("{base}{}", tc_cfg.upload_endpoint), raw, &headers).await?;
        if !status.is_success() {
            return Err(Error::Api {
                provider: "transcription_upload".into(),
                status_code: status.as_u16(),
                message: body,
            });
        }
        let up: Value = serde_json::from_str(&body)?;
        let uploaded = lookup_handle_field(&up, "upload_url");
        if uploaded.is_empty() {
            return Err(Error::Unsupported(
                "transcription upload: response carried no upload_url".into(),
            ));
        }
        uploaded
    } else {
        url
    };

    let mut submit_headers = headers.clone();
    submit_headers.push(("content-type".into(), "application/json".into()));
    let (status, body) = post_json(
        &format!("{base}{}", tc_cfg.submit_endpoint),
        json!({ "audio_url": audio_url }),
        &submit_headers,
    )
    .await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: "transcription_submit".into(),
            status_code: status.as_u16(),
            message: body,
        });
    }
    let raw: Value = serde_json::from_str(&body)?;
    let id = lookup_handle_field(&raw, tc_cfg.submit_handle_field);
    if id.is_empty() {
        return Err(Error::Unsupported(format!(
            "transcription submit: empty handle field {:?}",
            tc_cfg.submit_handle_field
        )));
    }
    Ok(TranscriptionHandle {
        id,
        provider: provider.clone(),
    })
}

/// Polls the provider until the transcription job reaches a terminal state,
/// then returns the finished [`TranscriptionResponse`]. A status=error job
/// surfaces as an error (never a silent empty success). The status-to-terminal
/// mapping is read from config (STT-005); only result extraction is
/// wire-shape-keyed. The handle carries the transcript id and provider config,
/// so wait works across process boundaries. `poll` is configurable so tests can
/// shrink the interval (mirrors Go's package vars).
pub async fn wait_transcription(
    handle: &TranscriptionHandle,
    poll: TranscriptionPoll,
) -> Result<TranscriptionResponse, Error> {
    let mut adapter = new_transcription_adapter(handle)?;
    // The TranscriptionPoll cadence (tests shrink it) drives the engine loop.
    adapter.lc.poll_interval = poll.interval;
    adapter.lc.poll_timeout = poll.timeout;
    poll_job(&adapter).await
}

/// Binds async transcription to the job engine's four seams. `classify` uses the
/// config-backed default (status vs done_status / error_status); `result`
/// decodes the finished transcript per wire shape (no second hop).
struct TranscriptionAdapter {
    lc: LifecycleConfig,
    headers: Vec<(String, String)>,
    poll_url: String,
    tc_cfg: &'static TranscriptionDef,
}

impl JobAdapter for TranscriptionAdapter {
    type Out = TranscriptionResponse;

    fn config(&self) -> &LifecycleConfig {
        &self.lc
    }

    async fn poll(&self) -> Result<PollBody, Error> {
        let (status, body) = get_text(&self.poll_url, &self.headers).await?;
        if !status.is_success() {
            return Err(Error::Api {
                provider: "transcription_poll".into(),
                status_code: status.as_u16(),
                message: body,
            });
        }
        let raw: Value = serde_json::from_str(&body)?;
        Ok(PollBody::new(raw))
    }

    fn classify(&self, body: &PollBody) -> Result<Classification, Error> {
        Ok(classify_by_config(&self.lc, body))
    }

    async fn result(&self, body: &PollBody) -> Result<TranscriptionResponse, Error> {
        transcription_result(self.tc_cfg, body.value())
    }
}

/// Assembles the transcription adapter + its LifecycleConfig from today's
/// transcription facts. The status-to-terminal mapping stays config (status_path
/// / done_status / error_status, STT-005); the provider error message rides on
/// `cfg.error_message_path` so `wait` still surfaces it (S02).
fn new_transcription_adapter(
    handle: &TranscriptionHandle,
) -> Result<TranscriptionAdapter, Error> {
    let provider = handle.provider.clone();
    let tc_cfg = transcription_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support transcription", provider.name),
    })?;
    let cfg = provider_config(provider.name);
    let base = transcription_base_url(&provider, cfg);
    let headers = build_auth_headers(&provider, cfg);
    let poll_url = format!("{base}{}", tc_cfg.poll_endpoint.replace("{id}", &handle.id));

    let defaults = TranscriptionPoll::default();
    let lc = LifecycleConfig {
        noun: "transcription",
        provider: format!("{:?}", provider.name),
        id: handle.id.clone(),
        status_path: tc_cfg.status_path.to_string(),
        done_values: non_empty_values([tc_cfg.done_status]),
        error_values: non_empty_values([tc_cfg.error_status]),
        error_message_path: cfg.error_message_path.to_string(),
        poll_interval: defaults.interval,
        poll_timeout: defaults.timeout,
    };
    Ok(TranscriptionAdapter {
        lc,
        headers,
        poll_url,
        tc_cfg,
    })
}

pub(crate) async fn transcription_transcribe(
    b: Transcription,
    audio_parts: Vec<Part>,
) -> Result<TranscriptionResponse, Error> {
    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
    };
    let model = b.model.clone().unwrap_or_default();
    transcribe_sync(&provider, &model, audio_parts).await
}

/// Runs a SYNCHRONOUS speech-to-text request (ADR-051): one multipart/form-data
/// POST returns the transcript directly, no job handle. Pre-flight rejects a
/// non-sync provider (naming Submit/Wait), a missing model, a remote audio URL
/// (OpenAI ingests inline bytes only — the inverse of AssemblyAI, OAA-005), and
/// a non-single-audio-bytes input. Mirror of go transcribeSync.
pub async fn transcribe_sync(
    provider: &Provider,
    model: &str,
    parts: Vec<Part>,
) -> Result<TranscriptionResponse, Error> {
    validate_provider(provider)?;
    let tc_cfg = transcription_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support transcription", provider.name),
    })?;
    if tc_cfg.interaction != "sync" {
        return Err(Error::Validation {
            field: "interaction",
            message: format!(
                "{:?} transcribes asynchronously; use Submit/Wait, not Transcribe",
                provider.name
            ),
        });
    }
    if model.is_empty() {
        return Err(Error::Validation {
            field: "model",
            message: "required for synchronous transcription".into(),
        });
    }
    let media = normalize_audio_bytes_part(&parts)?;

    let cfg = provider_config(provider.name);
    let base = transcription_base_url(provider, cfg);
    let headers = build_auth_headers(provider, cfg);

    // Build the multipart body in FIXED field order (model, response_format,
    // file) so all four SDKs emit the same canonical descriptor. reqwest sets
    // the multipart Content-Type + boundary from the Form.
    let mime = if media.mime_type.is_empty() {
        "application/octet-stream".to_string()
    } else {
        media.mime_type.clone()
    };
    let filename = format!("audio.{}", audio_ext_for_mime(&media.mime_type));
    let file_part = reqwest::multipart::Part::bytes(media.bytes.clone())
        .file_name(filename)
        .mime_str(&mime)?;
    let form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .text("response_format", "verbose_json")
        .part("file", file_part);

    let (status, body) =
        post_multipart(&format!("{base}{}", tc_cfg.submit_endpoint), form, &headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: format!("{:?}", provider.name),
            status_code: status.as_u16(),
            message: body,
        });
    }
    let raw: Value = serde_json::from_str(&body)?;
    Ok(transcription_result_from_openai(&raw))
}

/// Extracts the transcript text and (when present) segment timings from a
/// synchronous OpenAI response. verbose_json offsets are SECONDS (float) ->
/// integer milliseconds (x1000, rounded, OAA-006). Models without segments[]
/// -> empty segments, not an error. Usage stays zero (OAA-007). Mirror of go
/// transcriptionResultFromOpenAI.
fn transcription_result_from_openai(raw: &Value) -> TranscriptionResponse {
    let text = raw
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    if let Some(segs) = raw.get("segments").and_then(|v| v.as_array()) {
        for sd in segs {
            if !sd.is_object() {
                continue;
            }
            let start = sd
                .get("start")
                .and_then(|v| v.as_f64())
                .map(|f| (f * 1000.0).round() as i64)
                .unwrap_or(0);
            let end = sd
                .get("end")
                .and_then(|v| v.as_f64())
                .map(|f| (f * 1000.0).round() as i64)
                .unwrap_or(0);
            segments.push(TranscriptSegment {
                text: sd.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                start,
                end,
                speaker: String::new(),
            });
        }
    }
    TranscriptionResponse {
        text,
        segments,
        ..TranscriptionResponse::default()
    }
}

/// Enforces the single-audio-part rule for the sync path (OAA-005): exactly one
/// inline-bytes audio Part. A remote URL is rejected (OpenAI ingests no URL —
/// the inverse of AssemblyAI). Mirror of go normalizeAudioBytesPart.
fn normalize_audio_bytes_part(parts: &[Part]) -> Result<crate::structs::MediaRef, Error> {
    let mut media: Option<crate::structs::MediaRef> = None;
    let mut audio_count = 0;
    for part in parts {
        match part {
            Part::AudioBytes(m) => {
                audio_count += 1;
                media = Some(m.clone());
            }
            Part::AudioUrl(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "synchronous transcription accepts inline audio bytes only (audio_bytes); a remote audio URL is not supported".into(),
                });
            }
            Part::Text(_) | Part::Image(_) | Part::Lyrics(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "transcription accepts only audio parts (audio_bytes)".into(),
                });
            }
        }
    }
    match media {
        Some(m) if audio_count == 1 => Ok(m),
        _ => Err(Error::Validation {
            field: "parts",
            message: "transcription requires exactly one audio part".into(),
        }),
    }
}

/// Maps an audio IANA media type to the file extension OpenAI uses to detect the
/// format. Mirror of go audioExtForMime.
fn audio_ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "audio/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/flac" => "flac",
        _ => "bin",
    }
}

/// Extension trait — adds `wait()` to TranscriptionHandle so the typed-builder
/// API can offer a method-style call site (mirrors `VideoHandleExt`).
#[allow(async_fn_in_trait)]
pub trait TranscriptionHandleExt {
    async fn wait(&self) -> Result<TranscriptionResponse, Error>;

    /// Performs exactly ONE provider round-trip and returns the normalized
    /// [`JobStatus`] (ADR-063 POLL-001) — the non-blocking primitive for callers
    /// driving their own poll loop. On a completed job `JobStatus.result`
    /// carries the finished [`TranscriptionResponse`]; a failed job populates
    /// `JobStatus.cause` (the provider error surfaces in `cause.message`,
    /// preserving the `wait` error surface). Safe on a reconstituted handle
    /// (ADR-014 cross-process resume; POLL-005).
    async fn poll(&self) -> Result<JobStatus<TranscriptionResponse>, Error>;
}

impl TranscriptionHandleExt for TranscriptionHandle {
    async fn wait(&self) -> Result<TranscriptionResponse, Error> {
        wait_transcription(self, TranscriptionPoll::default()).await
    }

    async fn poll(&self) -> Result<JobStatus<TranscriptionResponse>, Error> {
        let adapter = new_transcription_adapter(self)?;
        poll_once(&adapter).await
    }
}

// ADR-064 AJU-007: awaiting a TranscriptionHandle directly delegates to `wait`,
// so the blocking one-liner `c.transcription().submit(...).await?.await?` works.
// The synchronous `transcribe` terminal (ADR-051) is unaffected — it returns a
// result with no handle (AJU-006).
impl IntoFuture for TranscriptionHandle {
    type Output = Result<TranscriptionResponse, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.wait().await })
    }
}

/// Extracts the finished transcript per wire shape. Only the result decode is
/// wire-shape-keyed (STT-005); the submit/poll/status facts are config. Mirror
/// of go transcriptionResult.
fn transcription_result(
    tc_cfg: &TranscriptionDef,
    raw: &Value,
) -> Result<TranscriptionResponse, Error> {
    match tc_cfg.wire_shape {
        "TranscriptionAssemblyAI" => Ok(transcription_result_from_assemblyai(raw)),
        other => Err(Error::Unsupported(format!(
            "transcription: unsupported wire shape {other:?}"
        ))),
    }
}

/// Extracts the transcript text and word-level timing segments from a completed
/// AssemblyAI transcript object. start/end are integer milliseconds; speaker is
/// present only on diarized transcripts. Usage stays zero — AssemblyAI bills by
/// audio duration, not tokens (ADR-048 OQ-2). Mirror of go
/// transcriptionResultFromAssemblyAI.
fn transcription_result_from_assemblyai(raw: &Value) -> TranscriptionResponse {
    let text = raw
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    if let Some(words) = raw.get("words").and_then(|v| v.as_array()) {
        for w in words {
            if !w.is_object() {
                continue;
            }
            segments.push(TranscriptSegment {
                text: w.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                start: w.get("start").and_then(|v| v.as_i64()).unwrap_or(0),
                end: w.get("end").and_then(|v| v.as_i64()).unwrap_or(0),
                speaker: w
                    .get("speaker")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    TranscriptionResponse {
        text,
        segments,
        ..TranscriptionResponse::default()
    }
}

/// Enforces the single-audio-part rule (STT-003) and returns the audio source:
/// a URL XOR raw bytes. A request with a non-audio part, or with anything other
/// than exactly one audio part, is rejected pre-flight. Mirror of go
/// normalizeAudioPart.
fn normalize_audio_part(parts: &[Part]) -> Result<(String, Option<Vec<u8>>), Error> {
    let mut url = String::new();
    let mut bytes: Option<Vec<u8>> = None;
    let mut audio_count = 0;
    for part in parts {
        match part {
            Part::AudioUrl(u) => {
                audio_count += 1;
                url = u.clone();
            }
            Part::AudioBytes(media) => {
                audio_count += 1;
                bytes = Some(media.bytes.clone());
            }
            Part::Text(_) | Part::Image(_) | Part::Lyrics(_) => {
                return Err(Error::Validation {
                    field: "parts",
                    message: "transcription accepts only audio parts (audio / audio_bytes)".into(),
                });
            }
        }
    }
    if audio_count != 1 {
        return Err(Error::Validation {
            field: "parts",
            message: "transcription requires exactly one audio part".into(),
        });
    }
    Ok((url, bytes))
}

/// Resolves the base for the transcription API: an explicit per-client override
/// wins (tests point it at a mock; users at a proxy), else the provider's chat
/// base. Submit/poll/upload endpoints are always relative paths joined to this
/// base. Mirror of go transcriptionBaseURL.
fn transcription_base_url(provider: &Provider, cfg: &ProviderSpec) -> String {
    if let Some(b) = &provider.base_url {
        return b.clone();
    }
    cfg.base_url.to_string()
}

/// Descends a dotted path (e.g. "id", "status", "error") through the decoded
/// response, returning the leaf string or "" if a segment is missing.
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
    match cur {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// POSTs raw bytes with an `application/octet-stream` body (the AssemblyAI
/// upload hop). The shared `post_json` helper sends JSON, so the upload builds
/// its reqwest request inline (the same shape as http.rs, with the auth headers
/// applied and content-type forced to octet-stream).
async fn post_octet_stream(
    url: &str,
    body: Vec<u8>,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = reqwest::Client::new();
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}
