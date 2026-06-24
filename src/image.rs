//! Image generation runtime — mirror of go/image.go.
//!
//! Pre-flight validation rejects unsupported aspect ratios, sizes, and
//! reference-image counts before any HTTP call. Dispatch branches on
//! img_cfg.input_mode (InlineParts → Google generateContent;
//! MultipartForm → OpenAI Image API with /generations vs /edits picked
//! dynamically per call based on whether image parts are present).

use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::error::Error;
use crate::http::{post_json, post_multipart};
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::paths::extract_u32_path;
use crate::providers::generated::image_gen::{image_gen_config, ImageGenDef, ImageModelDef};
use crate::providers::generated::providers::{provider_config, ProviderName};
use crate::request::build_auth_headers;
use crate::structs::ImageResponse;
pub use crate::structs::MediaRef;
use crate::types::{Provider, SafetySetting, Usage};
use crate::AuthScheme;

/// Universal multimodal input atom. The discriminator is compile-time
/// here (Rust enums) — no runtime XOR check needed at the Part level.
/// Use the `text` and `image` constructors for ergonomics.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Text(String),
    Image(MediaRef),
    /// Lyrics conditioning for music generation (ADR-033). Carried as a
    /// distinct variant from `Text` so the music runtime can route it to
    /// the provider's lyrics field (MiniMax) or fold it into the prompt
    /// (Gemini, and instrumental-only Vertex Lyria 2 per ADR-037 MUS-008).
    Lyrics(String),
}

impl Part {
    /// Construct a text Part.
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text(s.into())
    }

    /// Construct an image Part. `mime` is the IANA media type
    /// (e.g., "image/png"); `bytes` is raw (not base64-encoded).
    pub fn image(mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Part::Image(MediaRef {
            mime_type: mime.into(),
            bytes: bytes.into(),
        })
    }

    /// Construct a lyrics Part for music generation (ADR-033).
    pub fn lyrics(s: impl Into<String>) -> Self {
        Part::Lyrics(s.into())
    }

    /// True when this Part carries an image MediaRef.
    pub fn is_image(&self) -> bool {
        matches!(self, Part::Image(_))
    }
}

pub use crate::structs::ImageData;

/// Image-generation request.
///
/// Input is provided in one of two mutually-exclusive forms:
///   - `prompt`: terse sugar for the text-only hot path. Internally
///     desugars to `parts: vec![Part::text(prompt)]` before serialisation.
///   - `parts`: canonical multimodal sequence; required for editing and
///     compositional generation where caller-controlled ordering matters.
///
/// Pre-flight validation requires exactly one of `prompt` or `parts` to
/// be non-empty (XOR). Image-typed parts respect `img_cfg.max_input_count`.
#[derive(Clone, Debug, Default)]
pub struct ImageRequest {
    pub model: String,
    pub prompt: String,
    pub parts: Vec<Part>,
}

#[derive(Clone, Default)]
pub struct ImageOptions {
    pub aspect_ratio: Option<String>,
    pub image_size: Option<String>,
    pub include_text: bool,
    /// OpenAI gpt-image-* quality enum (low|medium|high|auto).
    pub quality: Option<String>,
    /// OpenAI gpt-image-* output MIME format (png|webp|jpeg).
    pub output_format: Option<String>,
    /// OpenAI gpt-image-* background treatment (transparent|opaque|auto).
    pub background: Option<String>,
    /// Number of images to generate; wire field `n`. OpenAI + xAI Grok.
    pub count: Option<u32>,
    /// PNG mask indicating which pixels to edit (transparent regions
    /// replaced). OpenAI gpt-image-* /v1/images/edits only.
    pub mask: Option<MediaRef>,
    /// Vertex Imagen safety filter threshold. Maps to
    /// `parameters.safetySetting` in the JSONPredict wire body.
    /// Constants: `IMAGE_SAFETY_FILTER_BLOCK_FEW/SOME/MOST/ONLY_HIGH`.
    pub safety_filter: Option<String>,
    /// Per-category safety thresholds for Google image generation (safetySettings[]).
    /// Same wire field as text-gen. ValidationError on non-Google image-gen providers.
    pub safety_settings: Vec<SafetySetting>,
    /// Free-form extras spread into the wire body. Reserved for provider
    /// knobs that don't yet have typed chain methods (OpenAI:
    /// output_compression, moderation). Knobs covered by typed methods
    /// (quality, output_format, background, count) are validated per
    /// provider; extra_fields is not.
    pub extra_fields: HashMap<String, Value>,
    pub middleware: Vec<MiddlewareFn>,
    /// Opt-in: populate `ImageResponse.raw` with the parsed provider
    /// response body (ADR-014). Plumbed by the typed builder's
    /// `.raw()` chain method.
    pub raw: bool,
}

impl std::fmt::Debug for ImageOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageOptions")
            .field("aspect_ratio", &self.aspect_ratio)
            .field("image_size", &self.image_size)
            .field("include_text", &self.include_text)
            .field("quality", &self.quality)
            .field("output_format", &self.output_format)
            .field("background", &self.background)
            .field("count", &self.count)
            .field("mask", &self.mask)
            .field("safety_filter", &self.safety_filter)
            .field("safety_settings", &self.safety_settings)
            .field("extra_fields", &self.extra_fields)
            .field("middleware", &format!("[{} fns]", self.middleware.len()))
            .field("raw", &self.raw)
            .finish()
    }
}

// ImageResponse is declared in rust/src/structs.rs (ADR-018, API-PDS-002).

pub async fn generate_image(
    provider: &Provider,
    request: &ImageRequest,
    options: &ImageOptions,
) -> Result<ImageResponse, Error> {
    if provider.api_key.is_empty() {
        return Err(Error::Validation {
            field: "api_key",
            message: "required".into(),
        });
    }
    if request.model.is_empty() {
        return Err(Error::Validation {
            field: "model",
            message: "required for image generation".into(),
        });
    }

    let parts = normalize_image_parts(request)?;

    let img_cfg = image_gen_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("{:?} does not support image generation", provider.name),
    })?;
    let model = find_image_model(img_cfg, &request.model).ok_or_else(|| Error::Validation {
        field: "model",
        message: format!(
            "{} is not a known image-generation model for {:?}",
            request.model, provider.name
        ),
    })?;

    // Empty whitelist means "no client-side check; pass through" — used
    // by providers (e.g., OpenAI) that accept arbitrary sizes within
    // documented bounds (plan 020 q1).
    if let Some(ratio) = &options.aspect_ratio {
        if !model.aspect_ratios.is_empty() && !model.aspect_ratios.contains(&ratio.as_str()) {
            return Err(Error::Validation {
                field: "aspect_ratio",
                message: format!("{} not supported by {}", ratio, request.model),
            });
        }
    }
    if let Some(size) = &options.image_size {
        if !model.image_sizes.is_empty() && !model.image_sizes.contains(&size.as_str()) {
            return Err(Error::Validation {
                field: "image_size",
                message: format!("{} not supported by {}", size, request.model),
            });
        }
    }
    let image_count = parts.iter().filter(|p| p.is_image()).count();
    if image_count > img_cfg.max_input_count {
        return Err(Error::Validation {
            field: "parts",
            message: format!(
                "{} image parts exceeds maximum {} for {:?}",
                image_count, img_cfg.max_input_count, provider.name
            ),
        });
    }

    // Per-provider knob validation. Quality/output_format/background are
    // OpenAI-only on the wire; count (n) is OpenAI + xAI; mask is OpenAI
    // edits-only. Mirrors go/image.go.
    if img_cfg.input_mode == "InlineParts" {
        if options.quality.is_some() {
            return Err(Error::Validation {
                field: "quality",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.output_format.is_some() {
            return Err(Error::Validation {
                field: "output_format",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.background.is_some() {
            return Err(Error::Validation {
                field: "background",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.count.is_some() {
            return Err(Error::Validation {
                field: "count",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.mask.is_some() {
            return Err(Error::Validation {
                field: "mask",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.safety_filter.is_some() {
            return Err(Error::Validation {
                field: "safety_filter",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        // safety_settings valid for InlineParts (Google); wired in build_image_body
    } else if img_cfg.input_mode == "JSONInlineRefs" {
        if options.quality.is_some() {
            return Err(Error::Validation {
                field: "quality",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.output_format.is_some() {
            return Err(Error::Validation {
                field: "output_format",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.background.is_some() {
            return Err(Error::Validation {
                field: "background",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.mask.is_some() {
            return Err(Error::Validation {
                field: "mask",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.safety_filter.is_some() {
            return Err(Error::Validation {
                field: "safety_filter",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if !options.safety_settings.is_empty() {
            return Err(Error::Validation {
                field: "safety_settings",
                message: format!("not supported by {:?}", provider.name),
            });
        }
    } else if img_cfg.input_mode == "MultipartForm" {
        if options.mask.is_some() && image_count == 0 {
            return Err(Error::Validation {
                field: "mask",
                message: "requires at least one image part (edits branch only)".into(),
            });
        }
        if options.safety_filter.is_some() {
            return Err(Error::Validation {
                field: "safety_filter",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if !options.safety_settings.is_empty() {
            return Err(Error::Validation {
                field: "safety_settings",
                message: format!("not supported by {:?}", provider.name),
            });
        }
    } else if img_cfg.input_mode == "JSONPredict" {
        if options.quality.is_some() {
            return Err(Error::Validation {
                field: "quality",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.output_format.is_some() {
            return Err(Error::Validation {
                field: "output_format",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.background.is_some() {
            return Err(Error::Validation {
                field: "background",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if !options.safety_settings.is_empty() {
            return Err(Error::Validation {
                field: "safety_settings",
                message: format!(
                    "not supported by {:?}; use safety_filter for Vertex Imagen",
                    provider.name
                ),
            });
        }
    } else if img_cfg.input_mode == "JSONGenerations" {
        // Recraft (text-to-image only). The flat generations body carries
        // only size (-> `size`) and count (-> `n`); aspect_ratio is not a
        // Recraft wire field (it sizes by an explicit WxH `size`), and the
        // gpt-image / safety knobs are OpenAI / Google / Vertex only. Image
        // parts are rejected upstream by the max_input_count==0 gate.
        if options.aspect_ratio.is_some() {
            return Err(Error::Validation {
                field: "aspect_ratio",
                message: format!(
                    "not supported by {:?}; use image_size (Recraft sizes by WxH)",
                    provider.name
                ),
            });
        }
        if options.quality.is_some() {
            return Err(Error::Validation {
                field: "quality",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.output_format.is_some() {
            return Err(Error::Validation {
                field: "output_format",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.background.is_some() {
            return Err(Error::Validation {
                field: "background",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.mask.is_some() {
            return Err(Error::Validation {
                field: "mask",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if options.safety_filter.is_some() {
            return Err(Error::Validation {
                field: "safety_filter",
                message: format!("not supported by {:?}", provider.name),
            });
        }
        if !options.safety_settings.is_empty() {
            return Err(Error::Validation {
                field: "safety_settings",
                message: format!("not supported by {:?}", provider.name),
            });
        }
    }

    let cfg = provider_config(provider.name);
    let base_event = Event {
        op: MiddlewareOp::ImageGeneration,
        provider: format!("{:?}", provider.name),
        model: request.model.clone(),
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(&options.middleware, &base_event)?;

    let auth_headers = build_auth_headers(provider, cfg);

    let result = (async {
        let base_url = provider
            .base_url
            .clone()
            .unwrap_or_else(|| cfg.base_url.to_string());

        let has_images = parts.iter().any(|p| p.is_image());

        let (status, response_body) = if img_cfg.input_mode == "JSONInlineRefs" {
            let body = if has_images {
                build_xai_edit_body(&parts, &request.model, options)
            } else {
                build_xai_gen_body(&parts, &request.model, options)
            };
            let endpoint = if has_images {
                img_cfg.edit_endpoint
            } else {
                img_cfg.gen_endpoint
            };
            let mut headers = auth_headers.clone();
            headers.push(("content-type".into(), "application/json".into()));
            post_json(&format!("{}{}", base_url, endpoint), body, &headers).await?
        } else if img_cfg.input_mode == "JSONGenerations" {
            let body = build_recraft_gen_body(&parts, &request.model, options);
            let mut headers = auth_headers.clone();
            headers.push(("content-type".into(), "application/json".into()));
            post_json(
                &format!("{}{}", base_url, img_cfg.gen_endpoint),
                body,
                &headers,
            )
            .await?
        } else if img_cfg.input_mode == "MultipartForm" {
            if has_images {
                let form = build_openai_edit_form(&parts, &request.model, options);
                post_multipart(
                    &format!("{}{}", base_url, img_cfg.edit_endpoint),
                    form,
                    &auth_headers,
                )
                .await?
            } else {
                let body = build_openai_gen_body(&parts, &request.model, options);
                let mut headers = auth_headers.clone();
                headers.push(("content-type".into(), "application/json".into()));
                post_json(
                    &format!("{}{}", base_url, img_cfg.gen_endpoint),
                    body,
                    &headers,
                )
                .await?
            }
        } else if img_cfg.input_mode == "JSONPredict" {
            let body = build_vertex_body(&parts, options);
            let endpoint = cfg.endpoint.replace("{model}", &request.model);
            let mut headers = auth_headers.clone();
            headers.push(("content-type".into(), "application/json".into()));
            post_json(&format!("{}{}", base_url, endpoint), body, &headers).await?
        } else {
            let body = build_image_body(&parts, options);
            let url = build_image_url(provider, cfg, &request.model);
            let mut headers = auth_headers.clone();
            headers.push(("content-type".into(), "application/json".into()));
            post_json(&url, body, &headers).await?
        };

        if !status.is_success() {
            return Err(Error::Api {
                provider: format!("{:?}", provider.name),
                status_code: status.as_u16(),
                message: response_body,
            });
        }
        let raw: Value = serde_json::from_str(&response_body)?;
        let mut parsed: ImageResponse = match provider.name {
            ProviderName::OpenAI => parse_image_response_data_array(
                &raw,
                "input_tokens",
                "output_tokens",
            ),
            // xAI reports usage.cost_in_usd_ticks rather than token counts;
            // empty field names yield zero tokens (correct, no fabricated values).
            ProviderName::Grok => parse_image_response_data_array(&raw, "", ""),
            // Recraft returns the same data[].b64_json shape as OpenAI/xAI
            // (the SDK forces response_format=b64_json) but carries no usage
            // object, so token fields are empty (zero tokens — no fabricated
            // values). SVG bytes (vector models) are sniffed to image/svg+xml
            // inside parse_image_response_data_array.
            ProviderName::Recraft => parse_image_response_data_array(&raw, "", ""),
            ProviderName::Vertex => parse_vertex_image_response(&raw),
            _ => {
                let (images, text, finish_reason, finish_message) =
                    extract_google_image_parts(&raw);
                let tokens = Usage {
                    input: extract_u32_path(&raw, cfg.usage_input_path),
                    output: extract_u32_path(&raw, cfg.usage_output_path),
                    ..Usage::default()
                };
                ImageResponse {
                    images,
                    text,
                    usage: tokens,
                    finish_reason,
                    finish_message,
                    raw: None,
                }
            }
        };
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

fn find_image_model<'a>(cfg: &'a ImageGenDef, model_id: &str) -> Option<&'a ImageModelDef> {
    cfg.models.iter().find(|m| m.model_id == model_id)
}

/// Enforce the XOR rule and produce the canonical `Vec<Part>` the rest
/// of the pipeline operates on. When only `prompt` is set (the text-only
/// sugar path), synthesise `vec![Part::text(prompt)]`. Both empty or both
/// set returns Error::Validation.
fn normalize_image_parts(request: &ImageRequest) -> Result<Vec<Part>, Error> {
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

fn build_image_body(parts: &[Part], options: &ImageOptions) -> Value {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut wire: Vec<Value> = Vec::with_capacity(parts.len());
    for part in parts {
        match part {
            // Image generation never carries lyrics parts (the music
            // runtime owns those); fold any here into text defensively.
            Part::Text(s) | Part::Lyrics(s) => wire.push(json!({ "text": s })),
            Part::Image(media) => wire.push(json!({
                "inlineData": {
                    "mimeType": media.mime_type,
                    "data": engine.encode(&media.bytes),
                }
            })),
        }
    }

    let modalities: Vec<&str> = if options.include_text {
        vec!["TEXT", "IMAGE"]
    } else {
        vec!["IMAGE"]
    };

    let mut generation_config = Map::new();
    generation_config.insert(
        "responseModalities".into(),
        Value::Array(
            modalities
                .into_iter()
                .map(|m| Value::String(m.into()))
                .collect(),
        ),
    );
    let mut image_config = Map::new();
    if let Some(ratio) = &options.aspect_ratio {
        image_config.insert("aspectRatio".into(), Value::String(ratio.clone()));
    }
    if let Some(size) = &options.image_size {
        image_config.insert("imageSize".into(), Value::String(size.clone()));
    }
    if !image_config.is_empty() {
        generation_config.insert("imageConfig".into(), Value::Object(image_config));
    }

    let mut body = json!({
        "contents": [{ "parts": wire }],
        "generationConfig": Value::Object(generation_config),
    });
    if !options.safety_settings.is_empty() {
        let ss: Vec<Value> = options
            .safety_settings
            .iter()
            .map(|s| json!({"category": s.category, "threshold": s.threshold}))
            .collect();
        body.as_object_mut()
            .unwrap()
            .insert("safetySettings".into(), Value::Array(ss));
    }
    body
}

/// Build the Vertex AI Imagen :predict request body.
///
/// Vertex uses an instances/parameters envelope: instance carries the
/// per-call inputs (prompt, image ref for editing, mask for inpainting);
/// parameters carries config (sampleCount, aspectRatio). Extra fields like
/// negativePrompt and safetySetting spread into parameters via
/// options.extra_fields so callers can reach Imagen-specific knobs without
/// typed chain methods.
fn build_vertex_body(parts: &[Part], options: &ImageOptions) -> Value {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut instance = Map::new();
    instance.insert("prompt".into(), Value::String(join_text_parts(parts)));
    for part in parts {
        if let Part::Image(media) = part {
            let mut img = Map::new();
            img.insert(
                "bytesBase64Encoded".into(),
                Value::String(engine.encode(&media.bytes)),
            );
            instance.insert("image".into(), Value::Object(img));
            break; // Vertex Imagen takes a single edit-target image
        }
    }
    if let Some(mask) = &options.mask {
        let mut mask_img = Map::new();
        mask_img.insert(
            "bytesBase64Encoded".into(),
            Value::String(engine.encode(&mask.bytes)),
        );
        let mut mask_obj = Map::new();
        mask_obj.insert("image".into(), Value::Object(mask_img));
        instance.insert("mask".into(), Value::Object(mask_obj));
    }

    let mut parameters = Map::new();
    parameters.insert(
        "sampleCount".into(),
        Value::Number(options.count.unwrap_or(1).into()),
    );
    if let Some(ratio) = &options.aspect_ratio {
        parameters.insert("aspectRatio".into(), Value::String(ratio.clone()));
    }
    if let Some(sf) = &options.safety_filter {
        parameters.insert("safetySetting".into(), Value::String(sf.clone()));
    }
    for (k, v) in &options.extra_fields {
        parameters.insert(k.clone(), v.clone());
    }

    json!({
        "instances": [Value::Object(instance)],
        "parameters": Value::Object(parameters),
    })
}

/// Decode Vertex AI Imagen :predict responses. Shape:
/// `{predictions: [{bytesBase64Encoded, mimeType}]}`. Vertex does not
/// return token counts in the predict response so Usage stays zero.
fn parse_vertex_image_response(raw: &Value) -> ImageResponse {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut images = Vec::new();
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
            let Some(b64) = entry.get("bytesBase64Encoded").and_then(|v| v.as_str()) else {
                continue;
            };
            if b64.is_empty() {
                continue;
            }
            let mime = entry
                .get("mimeType")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("image/png")
                .to_string();
            let Ok(decoded) = engine.decode(b64) else {
                continue;
            };
            images.push(ImageData {
                mime_type: mime,
                bytes: decoded,
            });
        }
    }
    ImageResponse {
        images,
        text: String::new(),
        usage: Usage::default(),
        finish_reason,
        finish_message: String::new(),
        raw: None,
    }
}

fn build_image_url(provider: &Provider, cfg: &crate::ProviderSpec, model: &str) -> String {
    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| cfg.base_url.to_string());
    let mut endpoint = cfg.endpoint.replace("{model}", model);
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

/// JSON body for /v1/images/generations.
///
/// Note: gpt-image-* models always return base64-encoded images via
/// `data[i].b64_json` and reject the `response_format` parameter (it
/// belonged to the legacy dall-e-* surface). Don't set it.
fn build_openai_gen_body(parts: &[Part], model: &str, options: &ImageOptions) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), Value::String(model.into()));
    body.insert("prompt".into(), Value::String(join_text_parts(parts)));
    if let Some(size) = &options.image_size {
        body.insert("size".into(), Value::String(size.clone()));
    }
    if let Some(q) = &options.quality {
        body.insert("quality".into(), Value::String(q.clone()));
    }
    if let Some(f) = &options.output_format {
        body.insert("output_format".into(), Value::String(f.clone()));
    }
    if let Some(bg) = &options.background {
        body.insert("background".into(), Value::String(bg.clone()));
    }
    if let Some(n) = options.count {
        body.insert("n".into(), Value::Number(n.into()));
    }
    for (k, v) in &options.extra_fields {
        body.insert(k.clone(), v.clone());
    }
    Value::Object(body)
}

/// Multipart form for /v1/images/edits. Each image Part becomes one
/// image[] file in caller order; text Parts join into the ``prompt`` field.
fn build_openai_edit_form(
    parts: &[Part],
    model: &str,
    options: &ImageOptions,
) -> reqwest::multipart::Form {
    let mut form = reqwest::multipart::Form::new()
        .text("model", model.to_string())
        .text("prompt", join_text_parts(parts));
    if let Some(size) = &options.image_size {
        form = form.text("size", size.clone());
    }
    if let Some(q) = &options.quality {
        form = form.text("quality", q.clone());
    }
    if let Some(f) = &options.output_format {
        form = form.text("output_format", f.clone());
    }
    if let Some(bg) = &options.background {
        form = form.text("background", bg.clone());
    }
    if let Some(n) = options.count {
        form = form.text("n", n.to_string());
    }
    for (k, v) in &options.extra_fields {
        let s = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        form = form.text(k.clone(), s);
    }
    let mut idx = 0;
    for part in parts {
        if let Part::Image(media) = part {
            let mime = if media.mime_type.is_empty() {
                "image/png"
            } else {
                &media.mime_type
            };
            let ext = ext_from_mime(mime);
            let part_form = reqwest::multipart::Part::bytes(media.bytes.clone())
                .file_name(format!("image-{}{}", idx, ext))
                .mime_str(mime)
                .unwrap_or_else(|_| reqwest::multipart::Part::bytes(media.bytes.clone()));
            form = form.part("image[]", part_form);
            idx += 1;
        }
    }
    if let Some(mask) = &options.mask {
        let mime = if mask.mime_type.is_empty() {
            "image/png"
        } else {
            &mask.mime_type
        };
        let ext = ext_from_mime(mime);
        let mask_form = reqwest::multipart::Part::bytes(mask.bytes.clone())
            .file_name(format!("mask{}", ext))
            .mime_str(mime)
            .unwrap_or_else(|_| reqwest::multipart::Part::bytes(mask.bytes.clone()));
        form = form.part("mask", mask_form);
    }
    form
}

/// JSON body for xAI Grok /v1/images/generations.
/// image_size maps to `resolution` (xAI's name); aspect_ratio maps as-is.
/// response_format=b64_json is forced because xAI defaults to URL.
fn build_xai_gen_body(parts: &[Part], model: &str, options: &ImageOptions) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), Value::String(model.into()));
    body.insert("prompt".into(), Value::String(join_text_parts(parts)));
    body.insert(
        "response_format".into(),
        Value::String("b64_json".into()),
    );
    if let Some(ratio) = &options.aspect_ratio {
        body.insert("aspect_ratio".into(), Value::String(ratio.clone()));
    }
    if let Some(size) = &options.image_size {
        body.insert("resolution".into(), Value::String(size.clone()));
    }
    if let Some(n) = options.count {
        body.insert("n".into(), Value::Number(n.into()));
    }
    for (k, v) in &options.extra_fields {
        body.insert(k.clone(), v.clone());
    }
    Value::Object(body)
}

/// JSON body for xAI Grok /v1/images/edits. Single image part →
/// `image: {url: "data:..."}`; multiple → `images: [...]` in caller order.
fn build_xai_edit_body(parts: &[Part], model: &str, options: &ImageOptions) -> Value {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut body = match build_xai_gen_body(parts, model, options) {
        Value::Object(map) => map,
        _ => unreachable!("build_xai_gen_body always returns an object"),
    };
    let mut refs: Vec<Value> = Vec::new();
    for part in parts {
        if let Part::Image(media) = part {
            let mime = if media.mime_type.is_empty() {
                "image/png"
            } else {
                &media.mime_type
            };
            let data_url = format!("data:{};base64,{}", mime, engine.encode(&media.bytes));
            let mut entry = Map::new();
            entry.insert("url".into(), Value::String(data_url));
            refs.push(Value::Object(entry));
        }
    }
    match refs.len() {
        0 => {}
        1 => {
            body.insert("image".into(), refs.into_iter().next().unwrap());
        }
        _ => {
            body.insert("images".into(), Value::Array(refs));
        }
    }
    Value::Object(body)
}

/// JSON body for Recraft's text-to-image /v1/images/generations endpoint.
/// image_size maps to `size`; count maps to `n`. response_format is forced
/// to b64_json because Recraft defaults to URL delivery — forcing it keeps
/// the response shape uniform (data[].b64_json). Vector/SVG output is
/// selected by a vector model id (recraftv3_vector), not a body flag, so the
/// body shape is identical for raster and vector. Style and other Recraft-
/// specific knobs ride extra_fields.
fn build_recraft_gen_body(parts: &[Part], model: &str, options: &ImageOptions) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), Value::String(model.into()));
    body.insert("prompt".into(), Value::String(join_text_parts(parts)));
    body.insert(
        "response_format".into(),
        Value::String("b64_json".into()),
    );
    if let Some(size) = &options.image_size {
        body.insert("size".into(), Value::String(size.clone()));
    }
    if let Some(n) = options.count {
        body.insert("n".into(), Value::Number(n.into()));
    }
    for (k, v) in &options.extra_fields {
        body.insert(k.clone(), v.clone());
    }
    Value::Object(body)
}

/// Report whether the decoded image bytes are an SVG document. SVG is XML
/// text starting (after optional whitespace) with an XML prolog (<?xml) or
/// the root <svg element. Used to label vector-model output (Recraft)
/// correctly when the provider does not echo a mime type.
fn looks_like_svg(data: &[u8]) -> bool {
    let s = String::from_utf8_lossy(data);
    let s = s.trim_start();
    s.starts_with("<?xml") || s.starts_with("<svg")
}

fn join_text_parts(parts: &[Part]) -> String {
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

fn ext_from_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => ".png",
        "image/jpeg" | "image/jpg" => ".jpg",
        "image/webp" => ".webp",
        _ => ".bin",
    }
}

/// Walk the data[] array shape used by both OpenAI's and xAI's image
/// APIs. Decodes data[i].b64_json; honors data[i].mime_type when echoed
/// back (xAI does, OpenAI does not), defaulting to image/png. Pass
/// empty token-field names for providers that don't report counts (xAI
/// reports usage.cost_in_usd_ticks instead).
fn parse_image_response_data_array(
    raw: &Value,
    input_token_field: &str,
    output_token_field: &str,
) -> ImageResponse {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut images: Vec<ImageData> = Vec::new();
    let mut revised: Vec<String> = Vec::new();
    if let Some(data) = raw.get("data").and_then(|v| v.as_array()) {
        for entry in data {
            if let Some(b64) = entry.get("b64_json").and_then(|v| v.as_str()) {
                if !b64.is_empty() {
                    if let Ok(decoded) = engine.decode(b64) {
                        let mut mime = entry
                            .get("mime_type")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .unwrap_or("image/png")
                            .to_string();
                        // Vector providers (Recraft recraftv3_vector) return
                        // SVG bytes in the same b64_json slot without echoing
                        // a mime_type. Sniff the leading bytes so SVG is
                        // labeled image/svg+xml rather than the image/png
                        // default. Raster bytes (PNG/JPEG/WebP) never start
                        // with '<', so the sniff is a no-op for them.
                        if mime == "image/png" && looks_like_svg(&decoded) {
                            mime = "image/svg+xml".to_string();
                        }
                        images.push(ImageData {
                            mime_type: mime,
                            bytes: decoded,
                        });
                    }
                }
            }
            if let Some(rp) = entry.get("revised_prompt").and_then(|v| v.as_str()) {
                if !rp.is_empty() {
                    revised.push(rp.to_string());
                }
            }
        }
    }
    let usage = raw.get("usage");
    let read_field = |field: &str| -> u32 {
        if field.is_empty() {
            return 0;
        }
        usage
            .and_then(|u| u.get(field))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32
    };
    let tokens = Usage {
        input: read_field(input_token_field),
        output: read_field(output_token_field),
        ..Usage::default()
    };
    ImageResponse {
        images,
        text: revised.join("\n"),
        usage: tokens,
        ..ImageResponse::default()
    }
}

fn extract_google_image_parts(raw: &Value) -> (Vec<ImageData>, String, String, String) {
    let engine = base64::engine::general_purpose::STANDARD;
    let candidates = match raw.get("candidates").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return (Vec::new(), String::new(), String::new(), String::new()),
    };
    let first = &candidates[0];
    let finish_reason = first
        .get("finishReason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let finish_message = first
        .get("finishMessage")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parts = first
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array());
    let parts = match parts {
        Some(p) => p,
        None => return (Vec::new(), String::new(), finish_reason, finish_message),
    };

    let mut images = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();
    for part in parts {
        if let Some(inline) = part.get("inlineData") {
            let data = inline.get("data").and_then(|v| v.as_str()).unwrap_or("");
            let mime = inline
                .get("mimeType")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !data.is_empty() {
                if let Ok(decoded) = engine.decode(data) {
                    images.push(ImageData {
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
    (images, text_parts.join(""), finish_reason, finish_message)
}
