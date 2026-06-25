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
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::transcription_gen::{transcription_config, TranscriptionDef};
use crate::request::{build_auth_headers, validate_provider};
use crate::structs::{TranscriptionHandle, TranscriptionResponse, TranscriptSegment};
use crate::types::Provider;

use super::Transcription;

//
//
//
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(3);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(600);

///
///
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
    };
    submit_transcription(&provider, audio_parts).await
}

///
///
///
///
///
pub async fn submit_transcription(
    provider: &Provider,
    parts: Vec<Part>,
) -> Result<TranscriptionHandle, Error> {
    validate_provider(provider)?;

    let tc_cfg = transcription_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support transcription", provider.name),
    })?;

    let (url, bytes) = normalize_audio_part(&parts)?;

    let cfg = provider_config(provider.name);
    let base = transcription_base_url(provider, cfg);
    let headers = build_auth_headers(provider, cfg);

    //
    //
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

///
///
///
///
///
///
///
pub async fn wait_transcription(
    handle: &TranscriptionHandle,
    poll: TranscriptionPoll,
) -> Result<TranscriptionResponse, Error> {
    let provider = &handle.provider;
    let tc_cfg = transcription_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support transcription", provider.name),
    })?;
    let cfg = provider_config(provider.name);

    let base = transcription_base_url(provider, cfg);
    let headers = build_auth_headers(provider, cfg);
    let poll_url = format!("{base}{}", tc_cfg.poll_endpoint.replace("{id}", &handle.id));

    let deadline = std::time::Instant::now() + poll.timeout;
    loop {
        if std::time::Instant::now() > deadline {
            return Err(Error::Unsupported(format!(
                "transcription poll: timed out waiting for {}",
                handle.id
            )));
        }
        let (status, body) = get_text(&poll_url, &headers).await?;
        if !status.is_success() {
            return Err(Error::Api {
                provider: "transcription_poll".into(),
                status_code: status.as_u16(),
                message: body,
            });
        }
        let raw: Value = serde_json::from_str(&body)?;
        let poll_status = lookup_handle_field(&raw, tc_cfg.status_path);
        if poll_status == tc_cfg.done_status {
            return Ok(transcription_result(tc_cfg, &raw)?);
        }
        if poll_status == tc_cfg.error_status {
            let mut msg = lookup_handle_field(&raw, cfg.error_message_path);
            if msg.is_empty() {
                msg = "transcription failed".into();
            }
            return Err(Error::Unsupported(format!("transcription failed: {msg}")));
        }
        //
        tokio::time::sleep(poll.interval).await;
    }
}

///
///
#[allow(async_fn_in_trait)]
pub trait TranscriptionHandleExt {
    async fn wait(&self) -> Result<TranscriptionResponse, Error>;
}

impl TranscriptionHandleExt for TranscriptionHandle {
    async fn wait(&self) -> Result<TranscriptionResponse, Error> {
        wait_transcription(self, TranscriptionPoll::default()).await
    }
}

///
///
///
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

///
///
///
///
///
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

///
///
///
///
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

///
///
///
///
fn transcription_base_url(provider: &Provider, cfg: &ProviderSpec) -> String {
    if let Some(b) = &provider.base_url {
        return b.clone();
    }
    cfg.base_url.to_string()
}

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
