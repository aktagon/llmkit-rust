//! Speech generation (text-to-speech) runtime — mirror of go/speech.go
//! (ADR-049).
//!
//! Pre-flight validation (model + text or lines required; voice or speakers;
//! provider supports speech; model known; speakers and style the model takes)
//! runs before any HTTP call. Voices are free-form: there is no catalogue.
//! Four wire shapes: SpeechInworld, SpeechOpenAI, SpeechGemini and
//! SpeechMiniMax. Sync, single AudioData. Middleware fires
//! around the request with `MiddlewareOp::SpeechGeneration` (BUG-036).

use base64::Engine;
use serde_json::{json, Value};

use crate::error::Error;
use crate::http::post_json_bytes;
use crate::middleware::{fire_post, fire_pre, set_event_error, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::request::AuthScheme;
use crate::providers::generated::speech_gen::{speech_gen_config, SpeechGenDef, SpeechModelDef};
use crate::request::build_auth_headers;
use crate::structs::{AudioData, SpeechResponse};
use crate::types::{Provider, SpeechLine, SpeechSpeaker};

/// Text-to-speech request (ADR-049).
///
/// `text` is the single utterance to speak (single-turn, no Message/Role
/// wrapper — SPK-003) and is empty when `lines` are set; `voice` is a
/// free-form voice id; `model` is required. `speakers` and `lines` make a
/// multi-speaker request; `style` is an optional delivery style (BUG-091).
#[derive(Clone, Debug, Default)]
pub struct SpeechRequest {
    pub model: String,
    pub voice: String,
    pub text: String,
    pub style: String,
    pub speakers: Vec<SpeechSpeaker>,
    pub lines: Vec<SpeechLine>,
}

/// Synthesizes speech audio from text.
///
/// Internal helper — the public surface is `Speech::generate` in
/// `builders/speech.rs`.
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
    validate_speech_inputs(request)?;

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
    validate_speech_model(request, model)?;

    let cfg = provider_config(provider.name);

    let base_event = Event {
        op: MiddlewareOp::SpeechGeneration,
        provider: format!("{:?}", provider.name),
        model: request.model.clone(),
        ..Event::default()
    };
    let start = crate::clock::Instant::now();
    fire_pre(middleware, &base_event)?;

    let mut auth_headers = build_auth_headers(provider, cfg);
    auth_headers.push(("content-type".into(), "application/json".into()));

    let result = (async {
        // Speech base: explicit override > the config's distinct speech base
        // (MiniMax, HANDOFF-094) > chat base, the music base contract.
        let base_url = provider.base_url.clone().unwrap_or_else(|| {
            if sg_cfg.base_url.is_empty() {
                cfg.base_url.to_string()
            } else {
                sg_cfg.base_url.to_string()
            }
        });
        let endpoint = if sg_cfg.gen_endpoint.is_empty() {
            cfg.endpoint
        } else {
            sg_cfg.gen_endpoint
        };
        let mut url = if endpoint.starts_with("http") {
            endpoint.to_string()
        } else {
            format!("{base_url}{endpoint}")
        };
        // Query-param-key providers (Google) carry the key in the URL, as on
        // every other Google call; Error's From<reqwest::Error> drops the URL
        // (VULN-001).
        if matches!(crate::auth_scheme(provider.name), AuthScheme::QueryParamKey) {
            let separator = if url.contains('?') { "&" } else { "?" };
            url.push_str(separator);
            url.push_str(cfg.auth_query_param);
            url.push('=');
            url.push_str(&provider.api_key);
        }

        let body = match sg_cfg.wire_shape {
            "SpeechOpenAI" => build_openai_speech_body(request),
            "SpeechGemini" => build_gemini_speech_body(request),
            "SpeechMiniMax" => build_minimax_speech_body(request),
            _ => build_inworld_speech_body(request),
        };
        // Read raw bytes: the OpenAI shape returns binary audio (not JSON), so
        // the response body must not be lossily UTF-8 decoded before the
        // encoding fork.
        let (status, response_bytes) =
            post_json_bytes(&url, provider.timeout, body, &auth_headers).await?;
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
        Ok(resp) => post_event.usage = Some(resp.usage),
        Err(err) => set_event_error(&mut post_event, err),
    }
    fire_post(middleware, &post_event);
    result
}

fn invalid(field: &'static str, message: impl Into<String>) -> Error {
    Error::Validation {
        field,
        message: message.into(),
    }
}

/// Checks the request on its own, before the provider config is read
/// (BUG-091). The messages are the same in every SDK.
fn validate_speech_inputs(request: &SpeechRequest) -> Result<(), Error> {
    if request.lines.is_empty() && request.text.is_empty() {
        return Err(invalid("text", "required for speech generation"));
    }
    if !request.lines.is_empty() && !request.text.is_empty() {
        return Err(invalid("text", "must be empty when lines are set"));
    }
    if request.speakers.is_empty() {
        if request.voice.is_empty() {
            return Err(invalid("voice", "required for speech generation"));
        }
    } else {
        if !request.voice.is_empty() {
            return Err(invalid("voice", "cannot be set together with speakers"));
        }
        if !request.style.is_empty() {
            return Err(invalid(
                "style",
                "cannot be set together with speakers; style each line instead",
            ));
        }
        if request.lines.is_empty() {
            return Err(invalid("lines", "required when speakers are set"));
        }
    }
    for line in &request.lines {
        if !request.speakers.iter().any(|s| s.name == line.speaker) {
            return Err(invalid(
                "lines",
                format!("{} is not a declared speaker", line.speaker),
            ));
        }
    }
    Ok(())
}

/// Checks the request against the model's speaker and style facts (BUG-091).
fn validate_speech_model(request: &SpeechRequest, model: &SpeechModelDef) -> Result<(), Error> {
    if !request.speakers.is_empty() {
        if model.max_speakers == 0 {
            return Err(invalid(
                "speakers",
                format!("{} does not support multiple speakers", model.model_id),
            ));
        }
        if request.speakers.len() as i64 != model.max_speakers {
            return Err(invalid(
                "speakers",
                format!(
                    "exactly {} speakers are required, got {}",
                    model.max_speakers,
                    request.speakers.len()
                ),
            ));
        }
    }
    let styled = !request.style.is_empty() || request.lines.iter().any(|l| !l.style.is_empty());
    if styled && !model.supports_speech_style {
        return Err(invalid(
            "style",
            format!("{} does not support a speech style", model.model_id),
        ));
    }
    Ok(())
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
/// base64-decodes the audio field; "hexEnvelope" (MiniMax) hex-decodes
/// data.audio and raises on a nonzero base_resp. A 2xx body that does not parse to audio is
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
    } else if audio_encoding == "base64InteractionSteps" {
        audio = parse_interaction_steps_audio(provider_name, fallback_mime, body)?;
    } else if audio_encoding == "hexEnvelope" {
        audio = parse_hex_envelope_audio(provider_name, fallback_mime, body)?;
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

/// Assembles the MiniMax /v1/t2a_v2 request body (HANDOFF-094): only the
/// fields the reference marks required. stream, output_format (hex) and
/// audio_setting (mp3) take their defaults.
fn build_minimax_speech_body(request: &SpeechRequest) -> Value {
    json!({
        "model": request.model,
        "text": request.text,
        "voice_setting": {"voice_id": request.voice},
    })
}

/// Decodes a MiniMax t2a_v2 reply (HANDOFF-094):
/// {"data": {"audio": "<hex>"}, "base_resp": {"status_code", "status_msg"}}.
/// MiniMax reports failures in base_resp on an HTTP 200, so a nonzero
/// status_code is an API error carrying status_msg. A success with missing
/// audio, or invalid hex, is a decoding error (HANDOFF-036 A5).
fn parse_hex_envelope_audio(
    provider_name: &str,
    fallback_mime: &str,
    body: &[u8],
) -> Result<AudioData, Error> {
    let raw = serde_json::from_slice::<Value>(body).map_err(|err| {
        Error::Unsupported(format!(
            "{provider_name} speech response: not valid JSON: {err}"
        ))
    })?;
    if let Some(code) = raw
        .pointer("/base_resp/status_code")
        .and_then(Value::as_i64)
    {
        if code != 0 {
            let msg = raw
                .pointer("/base_resp/status_msg")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Err(Error::Api {
                provider: provider_name.to_string(),
                status_code: 200,
                message: format!("{msg} (base_resp.status_code {code})"),
            });
        }
    }
    let hex = raw
        .pointer("/data/audio")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::Unsupported(format!(
                "{provider_name} speech response: missing or empty data.audio"
            ))
        })?;
    let bytes = crate::music::hex_decode(hex).ok_or_else(|| {
        Error::Unsupported(format!(
            "{provider_name} speech response: invalid hex in data.audio"
        ))
    })?;
    Ok(AudioData {
        mime_type: fallback_mime.to_string(),
        bytes,
    })
}

/// Assembles the OpenAI /v1/audio/speech request body. Slice 1 fixes
/// response_format=mp3 (KISS); format selection is a later slice (ADR-051).
/// A style is sent as `instructions` (BUG-091), which gpt-4o-mini-tts accepts.
fn build_openai_speech_body(request: &SpeechRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        "input": request.text,
        "voice": request.voice,
        "response_format": "mp3",
    });
    if !request.style.is_empty() {
        body["instructions"] = json!(request.style);
    }
    body
}

/// Assembles the Gemini TTS request body on the Interactions API (BUG-091):
/// one user_input turn and an audio response. Single voice: one text item and
/// `speech_config [{voice}]`; a style annotates the text item. Multi-speaker:
/// one text item per line, each annotated with its speaker and optional style,
/// and `speech_config {speakers}`. `store` is always false: the Interactions
/// API keeps requests by default.
fn build_gemini_speech_body(request: &SpeechRequest) -> Value {
    let (content, speech_config) = if request.speakers.is_empty() {
        let mut item = json!({"type": "text", "text": request.text});
        if !request.style.is_empty() {
            item["annotations"] = json!([{"type": "speech_metadata", "style": request.style}]);
        }
        (vec![item], json!([{"voice": request.voice}]))
    } else {
        let content = request
            .lines
            .iter()
            .map(|line| {
                let mut meta = json!({"type": "speech_metadata", "speaker": line.speaker});
                if !line.style.is_empty() {
                    meta["style"] = json!(line.style);
                }
                json!({"type": "text", "text": line.text, "annotations": [meta]})
            })
            .collect::<Vec<_>>();
        let speakers = request
            .speakers
            .iter()
            .map(|s| json!({"speaker": s.name, "voice": s.voice}))
            .collect::<Vec<_>>();
        (content, json!({"speakers": speakers}))
    };
    json!({
        "model": request.model,
        "input": [{"type": "user_input", "content": content}],
        "response_format": {"type": "audio"},
        "generation_config": {"speech_config": speech_config},
        "store": false,
    })
}

/// Decodes a Gemini TTS Interactions API reply (BUG-091). Google's documented
/// path is
/// `[.steps[] | select(.type=="model_output") | .content[] | select(.type=="audio")] | last | .data`:
/// the LAST audio item across the model_output steps wins. Its mime_type is
/// used when present, else the model's outputMime. No audio item, or invalid
/// base64, is a decoding error (HANDOFF-036 A5).
fn parse_interaction_steps_audio(
    provider_name: &str,
    fallback_mime: &str,
    body: &[u8],
) -> Result<AudioData, Error> {
    let raw = serde_json::from_slice::<Value>(body).map_err(|err| {
        Error::Unsupported(format!(
            "{provider_name} speech response: not valid JSON: {err}"
        ))
    })?;
    let last = raw
        .get("steps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|step| step.get("type").and_then(Value::as_str) == Some("model_output"))
        .filter_map(|step| step.get("content").and_then(Value::as_array))
        .flatten()
        .rfind(|item| item.get("type").and_then(Value::as_str) == Some("audio"));
    let b64 = last
        .and_then(|item| item.get("data"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::Unsupported(format!(
                "{provider_name} speech response: no audio item in steps[model_output].content"
            ))
        })?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|err| {
            Error::Unsupported(format!(
                "{provider_name} speech response: invalid base64 in audio data: {err}"
            ))
        })?;
    let mime_type = last
        .and_then(|item| item.get("mime_type"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_mime)
        .to_string();
    Ok(AudioData { mime_type, bytes })
}
