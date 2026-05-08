//! Phase 3 slice 1 — wires Image::generate against legacy `generate_image`.

use crate::error::Error;
use crate::image::{ImageOptions, ImageRequest, ImageResponse, Part};
use crate::types::Provider;

use super::Image;

pub async fn image_generate(
    b: Image,
    msg: impl Into<String>,
) -> Result<ImageResponse, Error> {
    let final_text: String = msg.into();

    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
    };

    let mut request = ImageRequest {
        model: b.model.clone().unwrap_or_default(),
        prompt: String::new(),
        parts: Vec::new(),
    };

    // XOR rule: prompt or parts, never both. If chain accumulated parts,
    // append msg as a final text Part and use the parts path; otherwise
    // use the prompt sugar path.
    if !b.parts.is_empty() {
        let mut parts = b.parts.clone();
        if !final_text.is_empty() {
            parts.push(Part::text(final_text));
        }
        request.parts = parts;
    } else if !final_text.is_empty() {
        request.prompt = final_text;
    }

    let mut options = ImageOptions::default();
    if let Some(ref r) = b.aspect_ratio {
        options.aspect_ratio = Some(r.clone());
    }
    if let Some(ref s) = b.image_size {
        options.image_size = Some(s.clone());
    }
    if b.include_text {
        options.include_text = true;
    }
    if !b.middleware.is_empty() {
        options.middleware = b.middleware.clone();
    }

    crate::generate_image(&provider, &request, &options).await
}
