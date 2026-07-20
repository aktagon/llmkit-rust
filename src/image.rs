//!
//!
//!
//!
//!
//!
//!

use base64::Engine;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

use crate::error::Error;
use crate::http::{post_json, post_multipart};
use crate::middleware::{fire_post, fire_pre, set_event_error, Event, MiddlewareFn, MiddlewareOp};
use crate::paths::extract_u32_path;
use crate::providers::generated::image_gen::{image_gen_config, ImageGenDef, ImageModelDef};
use crate::providers::generated::providers::provider_config;
use crate::request::build_auth_headers;
use crate::structs::ImageResponse;
pub use crate::structs::MediaRef;
use crate::types::{Provider, SafetySetting, Usage};
use crate::AuthScheme;

///
///
///
#
pub enum Part {
    Text(String),
    Image(MediaRef),
    ///
    ///
    ///
    ///
    Lyrics(String),
    ///
    ///
    ///
    AudioUrl(String),
    ///
    ///
    AudioBytes(MediaRef),
}

impl Part {
    ///
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text(s.into())
    }

    ///
    ///
    pub fn image(mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Part::Image(MediaRef {
            mime_type: mime.into(),
            bytes: bytes.into(),
        })
    }

    ///
    pub fn lyrics(s: impl Into<String>) -> Self {
        Part::Lyrics(s.into())
    }

    ///
    ///
    ///
    pub fn audio(url: impl Into<String>) -> Self {
        Part::AudioUrl(url.into())
    }

    ///
    ///
    ///
    ///
    pub fn audio_bytes(mime: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Part::AudioBytes(MediaRef {
            mime_type: mime.into(),
            bytes: bytes.into(),
        })
    }

    ///
    pub fn is_image(&self) -> bool {
        matches!(self, Part::Image(_))
    }
}

pub use crate::structs::ImageData;

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
pub struct ImageRequest {
    pub model: String,
    pub prompt: String,
    pub parts: Vec<Part>,
}

#
pub struct ImageOptions {
    pub aspect_ratio: Option<String>,
    pub image_size: Option<String>,
    pub include_text: bool,
    ///
    pub quality: Option<String>,
    ///
    pub output_format: Option<String>,
    ///
    pub background: Option<String>,
    ///
    pub count: Option<u32>,
    ///
    ///
    pub mask: Option<MediaRef>,
    ///
    ///
    ///
    pub safety_filter: Option<String>,
    ///
    ///
    pub safety_settings: Vec<SafetySetting>,
    ///
    ///
    ///
    ///
    ///
    pub extra_fields: HashMap<String, Value>,
    pub middleware: Vec<MiddlewareFn>,
    ///
    ///
    ///
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

//

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

    //
    //
    //
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

    //
    //
    //
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
        //
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
        //
        //
        //
        //
        //
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
        //
        //
        //
        let mut parsed: ImageResponse = match img_cfg.response_shape {
            //
            //
            "DataArrayB64Json" => parse_image_response_data_array(
                &raw,
                img_cfg.usage_input_path,
                img_cfg.usage_output_path,
            ),
            "VertexPredictions" => parse_vertex_image_response(&raw),
            //
            _ => {
                let (images, text, finish_reason, finish_message) =
                    extract_google_image_parts(&raw);
                let tokens = Usage {
                    input: extract_u32_path(&raw, img_cfg.usage_input_path),
                    output: extract_u32_path(&raw, img_cfg.usage_output_path),
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
        Err(err) => set_event_error(&mut post_event, err),
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

///
///
///
///
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
            //
            //
            Part::Text(s) | Part::Lyrics(s) => wire.push(json!({ "text": s })),
            Part::Image(media) => wire.push(json!({
                "inlineData": {
                    "mimeType": media.mime_type,
                    "data": engine.encode(&media.bytes),
                }
            })),
            //
            //
            Part::AudioUrl(_) | Part::AudioBytes(_) => {}
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

///
///
///
///
///
///
///
///
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

///
///
///
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

///
///
///
///
///
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

///
///
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

///
///
///
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

///
///
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

///
///
///
///
///
///
///
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

///
///
///
///
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

///
///
///
///
///
fn parse_image_response_data_array(
    raw: &Value,
    input_path: &str,
    output_path: &str,
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
                        //
                        //
                        //
                        //
                        //
                        //
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
    let read_path = |path: &str| -> u32 {
        if path.is_empty() {
            return 0;
        }
        extract_u32_path(raw, path)
    };
    let tokens = Usage {
        input: read_path(input_path),
        output: read_path(output_path),
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
