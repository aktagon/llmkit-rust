//!
//!

use crate::error::Error;
use crate::speech::{generate_speech, SpeechRequest};
use crate::structs::SpeechResponse;
use crate::types::Provider;

use super::Speech;

pub(crate) async fn speech_generate(
    b: Speech,
    msg: impl Into<String>,
) -> Result<SpeechResponse, Error> {
    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
    };

    let request = SpeechRequest {
        model: b.model.clone().unwrap_or_default(),
        voice: b.voice.clone().unwrap_or_default(),
        text: msg.into(),
    };

    generate_speech(&provider, &request).await
}
