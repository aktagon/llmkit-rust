//! Wires Speech::generate against `generate_speech` (ADR-049). Mirror of
//! builders/music.rs.

use crate::error::Error;
use crate::speech::{generate_speech, preview_speech_request, SpeechRequest};
use crate::structs::SpeechResponse;
use crate::types::Provider;

use super::Speech;

pub(crate) async fn speech_generate(
    b: Speech,
    msg: impl Into<String>,
) -> Result<SpeechResponse, Error> {
    let (provider, request) = speech_inputs(&b, msg);
    generate_speech(&provider, &request, &b.middleware).await
}

/// ADR-106: the body `generate` would send, from the same inputs.
pub(crate) fn speech_preview_request(b: &Speech, msg: impl Into<String>) -> Result<String, Error> {
    let (provider, request) = speech_inputs(b, msg);
    preview_speech_request(&provider, &request)
}

/// The runtime inputs the builder's chain state maps to.
fn speech_inputs(b: &Speech, msg: impl Into<String>) -> (Provider, SpeechRequest) {
    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
        timeout: b.client.provider.timeout,
    };

    let request = SpeechRequest {
        model: b.model.clone().unwrap_or_default(),
        voice: b.voice.clone().unwrap_or_default(),
        text: msg.into(),
        style: b.style.clone().unwrap_or_default(),
        speakers: b.speakers.clone(),
        lines: b.lines.clone(),
    };

    (provider, request)
}
