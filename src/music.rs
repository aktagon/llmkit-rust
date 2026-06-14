//! Music generation runtime — mirror of go/music.go (ADR-033).
//!
//! Pre-flight validation rejects image parts and unknown models before any
//! HTTP call; lyrics support is advisory (ADR-037 MUS-008), not gated —
//! lyrics fold into the prompt for the Predict shape. Dispatch branches on
//! the provider config's `wire_shape` — never on the provider name — which
//! fully determines the request body, the response audio path, AND the
//! byte encoding (base64 vs hex):
//!
//!   - MusicShapePredict (Vertex Lyria): instances/parameters envelope to
//!     :predict; audio at predictions[].audioContent (base64 WAV).
//!   - MusicShapeGenerateContent (Gemini Lyria 3): prompt + lyrics fold
//!     into contents[0].parts[].text with responseModalities=["AUDIO"];
//!     audio at candidates[0].content.parts[].inlineData.data (base64).
//!   - MusicShapeMinimax: top-level model/prompt/lyrics/audio_setting to
//!     the absolute gen endpoint; audio at data.audio (hex).

use base64::Engine;
use serde_json::{json, Value};

use crate::error::Error;
use crate::http::post_json;
use crate::image::Part;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::music_gen::{music_gen_config, MusicGenDef, MusicModelDef};
use crate::providers::generated::providers::provider_config;
use crate::request::build_auth_headers;
use crate::structs::{AudioData, MusicResponse};
use crate::types::{Provider, Usage};
use crate::AuthScheme;

// Wire-shape discriminators (mirror the generated string constants).
const SHAPE_PREDICT: &str = "MusicPredict";
const SHAPE_MINIMAX: &str = "MusicMinimax";

/// Music-generation request (ADR-033).
///
/// Model is required: music-generation models are explicit choices and the
/// text-generation default does not generate audio.
///
/// Input is provided in one of two mutually-exclusive forms:
///   - `prompt`: terse sugar for the prompt-only hot path. Internally
///     desugars to `parts: vec![Part::text(prompt)]` before serialisation.
///   - `parts`: canonical sequence of text and lyrics parts. A music
///     request never carries image parts; the runtime rejects them.
///
/// Pre-flight validation requires exactly one of `prompt` or `parts` to be
/// non-empty (XOR). Lyrics parts are rejected for instrumental-only models.
#[derive(Clone, Debug, Default)]
pub struct MusicRequest {
    pub model: String,
    pub prompt: String,
    pub parts: Vec<Part>,
}

/// Configures [`generate_music`].
#[derive(Clone, Default)]
pub struct MusicOptions {
    pub middleware: Vec<MiddlewareFn>,
    /// Opt-in: populate [`MusicResponse::raw`] with the parsed provider
    /// response body (ADR-014). Plumbed by the typed builder's `.raw()`.
    pub raw: bool,
}

impl std::fmt::Debug for MusicOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MusicOptions")
            .field("middleware", &format!("[{} fns]", self.middleware.len()))
            .field("raw", &self.raw)
            .finish()
    }
}

// AudioData and MusicResponse are declared in rust/src/structs.rs
// (ADR-018, API-PDS-002).

/// Produces audio from a text prompt, optionally conditioned on lyrics.
/// Input is either `prompt` (sugar) or `parts` (canonical sequence) —
/// exactly one must be set. Pre-flight validation rejects image parts and
/// unknown models before any HTTP call; lyrics support is advisory (ADR-037),
/// not gated. Fires the `MusicGeneration` middleware op pre + post.
pub async fn generate_music(
    provider: &Provider,
    request: &MusicRequest,
    options: &MusicOptions,
) -> Result<MusicResponse, Error> {
    if provider.api_key.is_empty() {
        return Err(Error::Validation {
            field: "api_key",
            message: "required".into(),
        });
    }
    if request.model.is_empty() {
        return Err(Error::Validation {
            field: "model",
            message: "required for music generation".into(),
        });
    }

    let parts = normalize_music_parts(request)?;
    // The Go/TS/Python twins also enforce a per-part "exactly one of text or
    // lyrics" check; here the `Part` enum makes that unrepresentable, so the
    // only per-part guard left is the image-part rejection.
    for part in &parts {
        if let Part::Image(_) = part {
            return Err(Error::Validation {
                field: "parts",
                message: "music generation does not accept image parts".into(),
            });
        }
    }

    let mg_cfg = music_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support music generation", provider.name),
    })?;
    let model = find_music_model(mg_cfg, &request.model).ok_or_else(|| Error::Validation {
        field: "model",
        message: format!(
            "{} is not a known music-generation model for {:?}",
            request.model, provider.name
        ),
    })?;
    // ADR-037 (MUS-008): supports_lyrics is advisory metadata, not a gate.
    // Lyrics on an instrumental-only model fold into the prompt (for the
    // single-prompt Predict shape) and the model ignores or honors them.

    let cfg = provider_config(provider.name);
    let base_event = Event {
        op: MiddlewareOp::MusicGeneration,
        provider: format!("{:?}", provider.name),
        model: request.model.clone(),
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(&options.middleware, &base_event)?;

    let mut auth_headers = build_auth_headers(provider, cfg);
    auth_headers.push(("content-type".into(), "application/json".into()));

    let result = (async {
        let base_url = provider
            .base_url
            .clone()
            .unwrap_or_else(|| cfg.base_url.to_string());

        let (body, url) = match mg_cfg.wire_shape {
            SHAPE_PREDICT => {
                let endpoint = if mg_cfg.gen_endpoint.is_empty() {
                    cfg.endpoint
                } else {
                    mg_cfg.gen_endpoint
                };
                let endpoint = endpoint.replace("{model}", &request.model);
                (
                    build_vertex_music_body(&parts),
                    format!("{base_url}{endpoint}"),
                )
            }
            SHAPE_MINIMAX => {
                let url = if mg_cfg.gen_endpoint.starts_with("http") {
                    mg_cfg.gen_endpoint.to_string()
                } else {
                    format!("{base_url}{}", mg_cfg.gen_endpoint)
                };
                (build_minimax_music_body(&parts, &request.model), url)
            }
            _ => (
                build_gemini_music_body(&parts),
                build_music_url(provider, cfg, mg_cfg, &request.model),
            ),
        };

        let (status, response_body) = post_json(&url, body, &auth_headers).await?;
        if !status.is_success() {
            return Err(Error::Api {
                provider: format!("{:?}", provider.name),
                status_code: status.as_u16(),
                message: response_body,
            });
        }
        let raw: Value = serde_json::from_str(&response_body)?;
        let mut parsed = parse_music_response(mg_cfg.wire_shape, model.output_mime, &raw);
        if options.raw {
            parsed.raw = Some(raw);
        }
        Ok(parsed)
    })
    .await;

    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    match &result {
        Ok(resp) => post_event.usage = Some(usage_to_event(&resp.usage)),
        Err(err) => post_event.err = Some(err.to_string()),
    }
    fire_post(&options.middleware, &post_event);
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

fn find_music_model<'a>(cfg: &'a MusicGenDef, model_id: &str) -> Option<&'a MusicModelDef> {
    cfg.models.iter().find(|m| m.model_id == model_id)
}

/// Enforce the XOR rule and produce the canonical `Vec<Part>`. When only
/// `prompt` is set, synthesise `vec![Part::text(prompt)]`. Both empty or
/// both set returns Error::Validation.
fn normalize_music_parts(request: &MusicRequest) -> Result<Vec<Part>, Error> {
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

/// Vertex AI Lyria :predict body. Lyria 2 has no lyrics wire-slot, so any
/// lyrics parts fold into the prompt text (ADR-037 MUS-008); the instrumental
/// model ignores vocal content. instances/parameters envelope mirrors Imagen.
fn build_vertex_music_body(parts: &[Part]) -> Value {
    let mut prompt = join_prompt_text(parts);
    let lyrics = join_lyrics_text(parts);
    if !lyrics.is_empty() {
        prompt = if prompt.is_empty() {
            lyrics
        } else {
            format!("{prompt}\n{lyrics}")
        };
    }
    json!({
        "instances": [{ "prompt": prompt }],
        "parameters": { "sampleCount": 1 },
    })
}

/// Gemini generateContent body for Lyria 3. Text and lyrics parts both
/// serialise as {text} parts in caller order (Gemini takes custom lyrics
/// inline in the prompt text). responseModalities requests AUDIO output.
fn build_gemini_music_body(parts: &[Part]) -> Value {
    let wire: Vec<Value> = parts
        .iter()
        .map(|p| match p {
            Part::Lyrics(s) => json!({ "text": s }),
            Part::Text(s) => json!({ "text": s }),
            // Image parts are rejected pre-flight; never reached here.
            Part::Image(_) => json!({ "text": "" }),
        })
        .collect();
    json!({
        "contents": [{ "parts": wire }],
        "generationConfig": { "responseModalities": ["AUDIO"] },
    })
}

/// MiniMax /v1/music_generation body. Prompt parts join into `prompt`;
/// lyrics parts join into `lyrics`. output_format=hex returns hex-encoded
/// audio at data.audio.
fn build_minimax_music_body(parts: &[Part], model: &str) -> Value {
    let mut body = json!({
        "model": model,
        "prompt": join_prompt_text(parts),
        "output_format": "hex",
        "audio_setting": {
            "sample_rate": 44100,
            "bitrate": 128000,
            "format": "mp3",
        },
    });
    let lyrics = join_lyrics_text(parts);
    if !lyrics.is_empty() {
        body.as_object_mut()
            .unwrap()
            .insert("lyrics".into(), Value::String(lyrics));
    }
    body
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

fn join_lyrics_text(parts: &[Part]) -> String {
    let mut texts: Vec<&str> = Vec::new();
    for p in parts {
        if let Part::Lyrics(s) = p {
            if !s.is_empty() {
                texts.push(s);
            }
        }
    }
    texts.join("\n")
}

/// Substitute the per-call model into the provider's endpoint template
/// (Gemini reuses the main generateContent endpoint) and append the query
/// auth key for query-param-key providers (Google).
fn build_music_url(
    provider: &Provider,
    cfg: &crate::ProviderSpec,
    mg_cfg: &MusicGenDef,
    model: &str,
) -> String {
    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| cfg.base_url.to_string());
    let mut endpoint = if mg_cfg.gen_endpoint.is_empty() {
        cfg.endpoint.to_string()
    } else {
        mg_cfg.gen_endpoint.to_string()
    };
    endpoint = endpoint.replace("{model}", model);
    endpoint = endpoint.replace("{apiKey}", &provider.api_key);
    if matches!(crate::auth_scheme(provider.name), AuthScheme::QueryParamKey) {
        let separator = if endpoint.contains('?') { "&" } else { "?" };
        endpoint.push_str(separator);
        endpoint.push_str(cfg.auth_query_param);
        endpoint.push('=');
        endpoint.push_str(&provider.api_key);
    }
    format!("{base}{endpoint}")
}

/// Decode audio payloads per wire shape. Each shape's response diverges
/// enough (predictions[] vs candidates[] vs data.audio, base64 vs hex)
/// that a match is clearer than a generic walker.
fn parse_music_response(wire_shape: &str, fallback_mime: &str, raw: &Value) -> MusicResponse {
    match wire_shape {
        SHAPE_PREDICT => parse_vertex_music_response(raw, fallback_mime),
        SHAPE_MINIMAX => parse_minimax_music_response(raw, fallback_mime),
        _ => parse_gemini_music_response(raw, fallback_mime),
    }
}

/// Vertex Lyria :predict responses. Shape:
/// `{"predictions": [{"audioContent": "<base64>", "mimeType": "audio/wav"}]}`.
fn parse_vertex_music_response(raw: &Value, fallback_mime: &str) -> MusicResponse {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut audio = Vec::new();
    let mut finish_reason = String::new();
    if let Some(preds) = raw.get("predictions").and_then(|v| v.as_array()) {
        for entry in preds {
            if finish_reason.is_empty() {
                if let Some(rai) = entry.get("raiFilteredReason").and_then(|v| v.as_str()) {
                    if !rai.is_empty() {
                        finish_reason = rai.to_string();
                    }
                }
            }
            let b64 = entry
                .get("audioContent")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    entry
                        .get("bytesBase64Encoded")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                });
            let Some(b64) = b64 else { continue };
            let mime = entry
                .get("mimeType")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(fallback_mime)
                .to_string();
            if let Ok(decoded) = engine.decode(b64) {
                audio.push(AudioData {
                    mime_type: mime,
                    bytes: decoded,
                });
            }
        }
    }
    MusicResponse {
        audio,
        finish_reason,
        ..MusicResponse::default()
    }
}

/// Gemini responses. Walks candidates[0].content.parts, decoding each
/// inlineData audio part and concatenating text parts (generated lyrics).
fn parse_gemini_music_response(raw: &Value, fallback_mime: &str) -> MusicResponse {
    let engine = base64::engine::general_purpose::STANDARD;
    let candidates = match raw.get("candidates").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return MusicResponse::default(),
    };
    let first = &candidates[0];
    let finish_reason = first
        .get("finishReason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parts = first
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array());

    let mut audio = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();
    if let Some(parts) = parts {
        for part in parts {
            if let Some(inline) = part.get("inlineData") {
                let data = inline.get("data").and_then(|v| v.as_str()).unwrap_or("");
                let mime = inline
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(fallback_mime)
                    .to_string();
                if !data.is_empty() {
                    if let Ok(decoded) = engine.decode(data) {
                        audio.push(AudioData {
                            mime_type: mime,
                            bytes: decoded,
                        });
                    }
                }
            }
            if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    text_parts.push(text.to_string());
                }
            }
        }
    }
    MusicResponse {
        audio,
        text: text_parts.join(""),
        finish_reason,
        ..MusicResponse::default()
    }
}

/// MiniMax responses. Shape:
/// `{"data": {"audio": "<hex>"}, "base_resp": {"status_msg": "..."}}`.
fn parse_minimax_music_response(raw: &Value, fallback_mime: &str) -> MusicResponse {
    let mut audio = Vec::new();
    if let Some(h) = raw
        .get("data")
        .and_then(|d| d.get("audio"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        if let Some(decoded) = hex_decode(h) {
            audio.push(AudioData {
                mime_type: fallback_mime.to_string(),
                bytes: decoded,
            });
        }
    }
    let mut finish_message = String::new();
    if let Some(msg) = raw
        .get("base_resp")
        .and_then(|b| b.get("status_msg"))
        .and_then(|v| v.as_str())
    {
        if !msg.is_empty() && msg != "success" {
            finish_message = msg.to_string();
        }
    }
    MusicResponse {
        audio,
        finish_message,
        ..MusicResponse::default()
    }
}

/// Decode a hex string to bytes. Returns None on odd length or any
/// non-hex digit (matching Go's hex.DecodeString error → no audio).
/// Hand-rolled to avoid a `hex` crate dependency.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    // MiniMax's gen_endpoint is an absolute https URL, so the dispatch
    // branch and its byte path can't be exercised through a base_url
    // redirect (matching how Go uses a rewriting RoundTripper and TS mocks
    // global fetch). These unit tests cover the MiniMax body shape and the
    // hex-decode path directly, against the same fixtures the Go/TS suites
    // assert end-to-end.
    const FAKE_MP3: &[u8] = &[0xFF, 0xFB, 0x90, 0x00, b'm', b'p', b'3'];

    #[test]
    fn minimax_body_prompt_only_omits_lyrics() {
        let parts = vec![Part::text("lofi hip hop")];
        let body = build_minimax_music_body(&parts, "music-2.6");
        assert_eq!(body["model"], "music-2.6");
        assert_eq!(body["prompt"], "lofi hip hop");
        assert_eq!(body["output_format"], "hex");
        assert_eq!(body["audio_setting"]["sample_rate"], 44100);
        assert_eq!(body["audio_setting"]["bitrate"], 128000);
        assert_eq!(body["audio_setting"]["format"], "mp3");
        assert!(
            body.get("lyrics").is_none(),
            "prompt-only request must not carry a lyrics field"
        );
    }

    #[test]
    fn minimax_body_lyrics_part_builds_lyrics_field() {
        let parts = vec![Part::text("pop ballad"), Part::lyrics("[chorus] hold on")];
        let body = build_minimax_music_body(&parts, "music-2.6");
        assert_eq!(body["prompt"], "pop ballad");
        assert_eq!(body["lyrics"], "[chorus] hold on");
    }

    #[test]
    fn minimax_response_hex_round_trips_with_config_mime() {
        let raw = json!({
            "data": { "audio": hex_encode(FAKE_MP3) },
            "base_resp": { "status_code": 0, "status_msg": "success" },
        });
        let resp = parse_minimax_music_response(&raw, "audio/mpeg");
        assert_eq!(resp.audio.len(), 1);
        assert_eq!(resp.audio[0].bytes, FAKE_MP3);
        assert_eq!(resp.audio[0].mime_type, "audio/mpeg");
        // status_msg "success" is not surfaced as a finish message.
        assert_eq!(resp.finish_message, "");
    }

    #[test]
    fn minimax_response_surfaces_non_success_status_msg() {
        let raw = json!({
            "data": { "audio": "" },
            "base_resp": { "status_code": 1004, "status_msg": "invalid api key" },
        });
        let resp = parse_minimax_music_response(&raw, "audio/mpeg");
        assert_eq!(resp.audio.len(), 0);
        assert_eq!(resp.finish_message, "invalid api key");
    }

    #[test]
    fn hex_decode_rejects_odd_length_and_non_hex() {
        assert_eq!(hex_decode("ff"), Some(vec![0xff]));
        assert_eq!(hex_decode("FFFB"), Some(vec![0xff, 0xfb]));
        assert_eq!(hex_decode("f"), None);
        assert_eq!(hex_decode("zz"), None);
    }

    fn provider(name: crate::ProviderName) -> Provider {
        Provider {
            name,
            api_key: "k".into(),
            model: None,
            base_url: Some("http://unused".into()),
        }
    }

    // The image-part rejection fires before any HTTP call. The Music
    // builder exposes no .image() chain method, so this drives the
    // crate-internal free function directly with a hand-built request.
    #[tokio::test]
    async fn generate_music_rejects_image_part() {
        let req = MusicRequest {
            model: "lyria-3-pro-preview".into(),
            prompt: String::new(),
            parts: vec![Part::text("a song"), Part::image("image/png", vec![0x89u8])],
        };
        let result = generate_music(
            &provider(crate::ProviderName::Google),
            &req,
            &MusicOptions::default(),
        )
        .await;
        match result {
            Err(Error::Validation { field, .. }) => assert_eq!(field, "parts"),
            other => panic!("expected parts validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn generate_music_requires_model_at_runtime() {
        let req = MusicRequest::default();
        let result = generate_music(
            &provider(crate::ProviderName::Vertex),
            &req,
            &MusicOptions::default(),
        )
        .await;
        match result {
            Err(Error::Validation { field, .. }) => assert_eq!(field, "model"),
            other => panic!("expected model validation error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn generate_music_rejects_unknown_provider() {
        let req = MusicRequest {
            model: "whatever".into(),
            prompt: "x".into(),
            parts: Vec::new(),
        };
        // OpenAI has no music_gen config.
        let result = generate_music(
            &provider(crate::ProviderName::OpenAI),
            &req,
            &MusicOptions::default(),
        )
        .await;
        match result {
            Err(Error::Validation { field, .. }) => assert_eq!(field, "provider"),
            other => panic!("expected provider validation error, got {:?}", other),
        }
    }
}
