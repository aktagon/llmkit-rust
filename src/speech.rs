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
use crate::http::post_json;
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

    let body = build_inworld_speech_body(request);
    let (status, response_body) = post_json(&url, body, &auth_headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: format!("{:?}", provider.name),
            status_code: status.as_u16(),
            message: response_body,
        });
    }
    let raw: Value = serde_json::from_str(&response_body)?;
    Ok(parse_speech_response(sg_cfg.wire_shape, model.output_mime, &raw))
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

/// Decodes the synthesized audio. SpeechInworld:
/// `{"audioContent": "<base64>", "usage": {...}}`.
fn parse_speech_response(_wire_shape: &str, fallback_mime: &str, raw: &Value) -> SpeechResponse {
    let mut audio = AudioData {
        mime_type: fallback_mime.to_string(),
        bytes: Vec::new(),
    };
    if let Some(b64) = raw.get("audioContent").and_then(Value::as_str) {
        if !b64.is_empty() {
            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(b64) {
                audio.bytes = decoded;
            }
        }
    }
    SpeechResponse {
        audio,
        ..Default::default()
    }
}
