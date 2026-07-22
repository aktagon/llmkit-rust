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

use crate::error::Error;
use crate::http::post_json_bytes;
use crate::middleware::{fire_post, fire_pre, set_event_error, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::speech_gen::{speech_gen_config, SpeechGenDef, SpeechModelDef};
use crate::request::build_auth_headers;
use crate::structs::{AudioData, SpeechResponse};
use crate::types::{Provider, Usage};

///
///
///
///
///
#[derive(Clone, Debug, Default)]
pub struct SpeechRequest {
    pub model: String,
    pub voice: String,
    pub text: String,
}

///
///
///
///
pub async fn generate_speech(
    provider: &Provider,
    request: &SpeechRequest,
    middleware: &[MiddlewareFn],
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

    let base_event = Event {
        op: MiddlewareOp::SpeechGeneration,
        provider: format!("{:?}", provider.name),
        model: request.model.clone(),
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(middleware, &base_event)?;

    let mut auth_headers = build_auth_headers(provider, cfg);
    auth_headers.push(("content-type".into(), "application/json".into()));

    let result = (async {
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
        //
        //
        //
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
    })
    .await;

    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    match &result {
        Ok(resp) => post_event.usage = Some(usage_to_event(&resp.usage)),
        Err(err) => set_event_error(&mut post_event, err),
    }
    fire_post(middleware, &post_event);
    result
}

fn usage_to_event(u: &Usage) -> crate::middleware::Usage {
    crate::middleware::Usage {
        input: u.input as i64,
        output: u.output as i64,
        cache_write: u.cache_write as i64,
        cache_read: u.cache_read as i64,
        reasoning: u.reasoning as i64,
        cost: u.cost,
    }
}

fn find_speech_model<'a>(cfg: &'a SpeechGenDef, model_id: &str) -> Option<&'a SpeechModelDef> {
    cfg.models.iter().find(|m| m.model_id == model_id)
}

///
///
///
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

///
///
///
///
///
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
        //
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

///
///
fn build_openai_speech_body(request: &SpeechRequest) -> Value {
    json!({
        "model": request.model,
        "input": request.text,
        "voice": request.voice,
        "response_format": "mp3",
    })
}
