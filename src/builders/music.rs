//!
//!

use crate::error::Error;
use crate::image::Part;
use crate::music::{generate_music, MusicOptions, MusicRequest};
use crate::structs::MusicResponse;
use crate::types::Provider;

use super::Music;

pub(crate) async fn music_generate(
    b: Music,
    msg: impl Into<String>,
) -> Result<MusicResponse, Error> {
    let final_text: String = msg.into();

    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
    };

    let mut request = MusicRequest {
        model: b.model.clone().unwrap_or_default(),
        prompt: String::new(),
        parts: Vec::new(),
    };

    //
    //
    //
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

    generate_music(&provider, &request, &options).await
}
