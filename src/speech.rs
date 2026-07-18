//! Speech generation (text-to-speech) runtime — mirror of go/speech.go
//! (ADR-049).
//!
//! Pre-flight validation (model + text + voice required; provider supports
//! speech; model in catalogue; voice in catalogue) runs before any HTTP call.
//! One wire shape (SpeechInworld): a flat-JSON POST whose response carries
//! base64 audio at `audioContent`. Sync, single AudioData, no middleware.

use base64::Engine;
use serde_json::{json, Value};

use crate::error::Error;
use crate::http::post_json_bytes;
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::speech_gen::{speech_gen_config, SpeechGenDef, SpeechModelDef};
use crate::request::build_auth_headers;
use crate::structs::{AudioData, SpeechResponse};
use crate::types::Provider;

/// Text-to-speech request (ADR-049).
///
/// `text` is the single utterance to speak (single-turn, no Message/Role
/// wrapper — SPK-003); `voice` is the request-data selector validated
/// pre-flight against the provider's catalogue (SPK-004); `model` is required.
#[derive(Clone, Debug, Default)]
pub struct SpeechRequest {
    pub model: String,
    pub voice: String,
    pub text: String,
}

/// Synthesizes speech audio from text.
///
/// Internal helper — the public surface is `Speech::generate` in
/// `builders/speech.rs`.
pub async fn generate_speech(
    provider: &Provider,
    request: &SpeechRequest,
) -> Result<SpeechResponse, Error> {
    if provider.api_key.is_empty() {
        return Err(Error::Validation {
            field: "api_key",
            message: "required".into(),
        });
    }
    if request.model.is_empty() {
        return Err(Error::Validation {
            field: "model",
            message: "required for speech generation".into(),
        });
    }
    if request.text.is_empty() {
        return Err(Error::Validation {
            field: "text",
            message: "required for speech generation".into(),
        });
    }
    if request.voice.is_empty() {
        return Err(Error::Validation {
            field: "voice",
            message: "required for speech generation".into(),
        });
    }

    let sg_cfg = speech_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support speech generation", provider.name),
    })?;
    let model = find_speech_model(sg_cfg, &request.model).ok_or_else(|| Error::Validation {
        field: "model",
        message: format!(
            "{} is not a known speech-generation model for {:?}",
            request.model, provider.name
        ),
    })?;
    if !sg_cfg.voices.contains(&request.voice.as_str()) {
        return Err(Error::Validation {
            field: "voice",
            message: format!("{} is not a known voice for {:?}", request.voice, provider.name),
        });
    }

    let cfg = provider_config(provider.name);
    let mut auth_headers = build_auth_headers(provider, cfg);
    auth_headers.push(("content-type".into(), "application/json".into()));

    let base_url = provider
        .base_url
        .clone()
        .unwrap_or_else(|| cfg.base_url.to_string());
    let endpoint = if sg_cfg.gen_endpoint.is_empty() {
        cfg.endpoint
    } else {
        sg_cfg.gen_endpoint
    };
    let url = if endpoint.starts_with("http") {
        endpoint.to_string()
    } else {
        format!("{base_url}{endpoint}")
    };

    let body = if sg_cfg.wire_shape == "SpeechOpenAI" {
        build_openai_speech_body(request)
    } else {
        build_inworld_speech_body(request)
    };
    // Read raw bytes: the OpenAI shape returns binary audio (not JSON), so the
    // response body must not be lossily UTF-8 decoded before the encoding fork.
    let (status, response_bytes) = post_json_bytes(&url, body, &auth_headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: format!("{:?}", provider.name),
            status_code: status.as_u16(),
            message: String::from_utf8_lossy(&response_bytes).into_owned(),
        });
    }
    parse_speech_response(
        &format!("{:?}", provider.name),
        sg_cfg.audio_response_encoding,
        model.output_mime,
        &response_bytes,
    )
}

fn find_speech_model<'a>(cfg: &'a SpeechGenDef, model_id: &str) -> Option<&'a SpeechModelDef> {
    cfg.models.iter().find(|m| m.model_id == model_id)
}

/// Assembles the Inworld /tts/v1/voice request body. Slice 1 sends a fixed
/// audioConfig (LINEAR16/22050 -> WAV) and BALANCED delivery; format/sample-
/// rate selection is a later slice (ADR-049 OQ-5).
fn build_inworld_speech_body(request: &SpeechRequest) -> Value {
    json!({
        "text": request.text,
        "voiceId": request.voice,
        "modelId": request.model,
        "audioConfig": {
            "audioEncoding": "LINEAR16",
            "sampleRateHertz": 22050,
        },
        "deliveryMode": "BALANCED",
    })
}

/// Decodes the synthesized audio per the wire shape's audio response encoding
/// (ADR-051 OAA-002). "rawBody" (OpenAI) takes the response body verbatim as
/// the audio bytes; "base64Envelope" (Inworld) parses a JSON envelope and
/// base64-decodes the audio field. A 2xx body that does not parse to audio is
/// a decoding error (HANDOFF-036 A5) — never a silent empty clip.
fn parse_speech_response(
    provider_name: &str,
    audio_encoding: &str,
    fallback_mime: &str,
    body: &[u8],
) -> Result<SpeechResponse, Error> {
    let mut audio = AudioData {
        mime_type: fallback_mime.to_string(),
        bytes: Vec::new(),
    };
    if audio_encoding == "rawBody" {
        audio.bytes = body.to_vec();
    } else {
        // base64Envelope: {"audioContent": "<base64>", "usage": {...}}.
        let raw = serde_json::from_slice::<Value>(body).map_err(|err| {
            Error::Unsupported(format!(
                "{provider_name} speech response: not valid JSON: {err}"
            ))
        })?;
        let b64 = raw
            .get("audioContent")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::Unsupported(format!(
                    "{provider_name} speech response: missing or empty audioContent"
                ))
            })?;
        audio.bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|err| {
                Error::Unsupported(format!(
                    "{provider_name} speech response: invalid base64 in audioContent: {err}"
                ))
            })?;
    }
    Ok(SpeechResponse {
        audio,
        ..Default::default()
    })
}

/// Assembles the OpenAI /v1/audio/speech request body. Slice 1 fixes
/// response_format=mp3 (KISS); format selection is a later slice (ADR-051).
fn build_openai_speech_body(request: &SpeechRequest) -> Value {
    json!({
        "model": request.model,
        "input": request.text,
        "voice": request.voice,
        "response_format": "mp3",
    })
}
