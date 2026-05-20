//! Phase 3 slice 1 — wires Image::generate against legacy `generate_image`.

use crate::error::Error;
use crate::image::{ImageOptions, ImageRequest, Part};
use crate::structs::ImageResponse;
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
    if let Some(ref q) = b.quality {
        options.quality = Some(q.clone());
    }
    if let Some(ref f) = b.output_format {
        options.output_format = Some(f.clone());
    }
    if let Some(ref bg) = b.background {
        options.background = Some(bg.clone());
    }
    if let Some(n) = b.count {
        options.count = Some(n);
    }
    if let Some(ref m) = b.mask {
        options.mask = Some(m.clone());
    }
    if let Some(ref sf) = b.safety_filter {
        options.safety_filter = Some(sf.clone());
    }
    if !b.safety_settings.is_empty() {
        options.safety_settings = b.safety_settings.clone();
    }
    if !b.middleware.is_empty() {
        options.middleware = b.middleware.clone();
    }
    if !b.extra_fields.is_empty() {
        options.extra_fields = b.extra_fields.clone();
    }
    if b.raw {
        options.raw = true;
    }

    crate::image::generate_image(&provider, &request, &options).await
}
