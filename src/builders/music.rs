//! Wires Music::generate against `generate_music` (ADR-033). Mirror of
//! builders/image.rs.

use crate::error::Error;
use crate::image::Part;
use crate::music::{generate_music, preview_music_request, MusicOptions, MusicRequest};
use crate::structs::MusicResponse;
use crate::types::Provider;

use super::Music;

pub(crate) async fn music_generate(
    b: Music,
    msg: impl Into<String>,
) -> Result<MusicResponse, Error> {
    let (provider, request, options) = music_inputs(&b, msg);
    generate_music(&provider, &request, &options).await
}

/// ADR-106: the body `generate` would send, from the same inputs.
pub(crate) fn music_preview_request(b: &Music, msg: impl Into<String>) -> Result<String, Error> {
    let (provider, request, _) = music_inputs(b, msg);
    preview_music_request(&provider, &request)
}

/// The runtime inputs the builder's chain state maps to.
fn music_inputs(b: &Music, msg: impl Into<String>) -> (Provider, MusicRequest, MusicOptions) {
    let final_text: String = msg.into();

    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
        timeout: b.client.provider.timeout,
    };

    let mut request = MusicRequest {
        model: b.model.clone().unwrap_or_default(),
        prompt: String::new(),
        parts: Vec::new(),
    };

    // XOR rule: prompt or parts, never both. If the chain accumulated
    // parts (text/lyrics), append msg as a final text Part and use the
    // parts path; otherwise use the prompt sugar path.
    if !b.parts.is_empty() {
        let mut parts = b.parts.clone();
        if !final_text.is_empty() {
            parts.push(Part::text(final_text));
        }
        request.parts = parts;
    } else if !final_text.is_empty() {
        request.prompt = final_text;
    }

    let mut options = MusicOptions::default();
    if !b.middleware.is_empty() {
        options.middleware = b.middleware.clone();
    }
    if b.raw {
        options.raw = true;
    }

    (provider, request, options)
}
