//! Image generation runtime — mirror of go/image.go.
//!
//! Pre-flight validation rejects unsupported aspect ratios, sizes, and
//! reference-image counts before any HTTP call. The body shape matches
//! Google's generateContent endpoint; OpenAI/Vertex variants will dispatch
//! on cfg.input_mode when those land.

use base64::Engine;
use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::http::post_json;
use crate::paths::extract_u32_path;
use crate::providers::generated::image_gen::{image_gen_config, ImageGenDef, ImageModelDef};
use crate::providers::generated::providers::provider_config;
use crate::request::build_auth_headers;
use crate::types::{Provider, Usage};
use crate::AuthScheme;

#[derive(Clone, Debug, Default)]
pub struct ImageInput {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct ImageData {
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct ImageRequest {
    pub prompt: String,
    pub model: String,
    pub reference_images: Vec<ImageInput>,
}

#[derive(Clone, Debug, Default)]
pub struct ImageOptions {
    pub aspect_ratio: Option<String>,
    pub image_size: Option<String>,
    pub include_text: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ImageResponse {
    pub images: Vec<ImageData>,
    pub text: String,
    pub tokens: Usage,
}

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
    if request.prompt.is_empty() {
        return Err(Error::Validation {
            field: "prompt",
            message: "required".into(),
        });
    }
    if request.model.is_empty() {
        return Err(Error::Validation {
            field: "model",
            message: "required for image generation".into(),
        });
    }

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

    if let Some(ratio) = &options.aspect_ratio {
        if !model.aspect_ratios.contains(&ratio.as_str()) {
            return Err(Error::Validation {
                field: "aspect_ratio",
                message: format!("{} not supported by {}", ratio, request.model),
            });
        }
    }
    if let Some(size) = &options.image_size {
        if !model.image_sizes.contains(&size.as_str()) {
            return Err(Error::Validation {
                field: "image_size",
                message: format!("{} not supported by {}", size, request.model),
            });
        }
    }
    if request.reference_images.len() > img_cfg.max_input_count {
        return Err(Error::Validation {
            field: "reference_images",
            message: format!(
                "{} exceeds maximum {} for {:?}",
                request.reference_images.len(),
                img_cfg.max_input_count,
                provider.name
            ),
        });
    }

    let cfg = provider_config(provider.name);
    let body = build_image_body(request, options);
    let url = build_image_url(provider, cfg, &request.model);
    let mut headers = build_auth_headers(provider, cfg);
    headers.push((
        "content-type".into(),
        "application/json".into(),
    ));

    let (status, response_body) = post_json(&url, body, &headers).await?;
    if !status.is_success() {
        return Err(Error::Api {
            provider: format!("{:?}", provider.name),
            status_code: status.as_u16(),
            message: response_body,
        });
    }

    let raw: Value = serde_json::from_str(&response_body)?;
    let (images, text) = extract_google_image_parts(&raw);
    let tokens = Usage {
        input: extract_u32_path(&raw, cfg.usage_input_path),
        output: extract_u32_path(&raw, cfg.usage_output_path),
        ..Usage::default()
    };
    Ok(ImageResponse {
        images,
        text,
        tokens,
    })
}

fn find_image_model<'a>(cfg: &'a ImageGenDef, model_id: &str) -> Option<&'a ImageModelDef> {
    cfg.models.iter().find(|m| m.model_id == model_id)
}

fn build_image_body(request: &ImageRequest, options: &ImageOptions) -> Value {
    let engine = base64::engine::general_purpose::STANDARD;
    let mut parts: Vec<Value> = vec![json!({ "text": request.prompt })];
    for reference in &request.reference_images {
        parts.push(json!({
            "inlineData": {
                "mimeType": reference.mime_type,
                "data": engine.encode(&reference.data),
            }
        }));
    }

    let modalities: Vec<&str> = if options.include_text {
        vec!["TEXT", "IMAGE"]
    } else {
        vec!["IMAGE"]
    };

    let mut generation_config = Map::new();
    generation_config.insert(
        "responseModalities".into(),
        Value::Array(modalities.into_iter().map(|m| Value::String(m.into())).collect()),
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

    json!({
        "contents": [{ "parts": parts }],
        "generationConfig": Value::Object(generation_config),
    })
}

fn build_image_url(provider: &Provider, cfg: &crate::ProviderConfig, model: &str) -> String {
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

fn extract_google_image_parts(raw: &Value) -> (Vec<ImageData>, String) {
    let engine = base64::engine::general_purpose::STANDARD;
    let candidates = match raw.get("candidates").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return (Vec::new(), String::new()),
    };
    let parts = candidates[0]
        .get("content")
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array());
    let parts = match parts {
        Some(p) => p,
        None => return (Vec::new(), String::new()),
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
                        data: decoded,
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
    (images, text_parts.join(""))
}
