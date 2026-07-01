//! Wires Video::submit against `submit_video` and adds `VideoHandle::wait`
//! via the `VideoHandleExt` extension trait (ADR-034). Mirror of
//! builders/batch.rs (handle pattern) and builders/music.rs (submit seam).
//!
//! VideoHandle is an ontology-generated pure-data struct (ADR-018). The
//! `wait()` method lives on the extension trait so the data struct stays
//! generated while the poll loop stays hand-coded. `VideoHandle.raw`
//! (ADR-014) is honored automatically; cross-process resume callers set
//! the field on the struct before calling `wait()`.

use crate::error::Error;
use crate::image::Part;
use crate::structs::{VideoHandle, VideoResponse};
use crate::types::Provider;
use crate::video::{submit_video, VideoPoll, VideoRequest};

use super::Video;

pub(crate) async fn video_submit(
    b: Video,
    msg: impl Into<String>,
) -> Result<VideoHandle, Error> {
    let final_text: String = msg.into();

    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
    };

    let mut request = VideoRequest {
        model: b.model.clone().unwrap_or_default(),
        prompt: String::new(),
        parts: Vec::new(),
        output_uri: b.output_uri.clone().unwrap_or_default(),
    };

    // XOR rule: prompt or parts, never both. If the chain accumulated
    // parts (via .text()), append msg as a trailing text Part and use the
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

    submit_video(&provider, &request, &b.middleware, b.raw).await
}

/// Extension trait — adds `wait()` to VideoHandle so the typed-builder API
/// can offer a method-style call site. `VideoHandle.raw` (ADR-014) is
/// honored automatically; cross-process resume callers set the field on the
/// struct before calling `wait()`.
#[allow(async_fn_in_trait)]
pub trait VideoHandleExt {
    async fn wait(&self) -> Result<VideoResponse, Error>;
}

impl VideoHandleExt for VideoHandle {
    async fn wait(&self) -> Result<VideoResponse, Error> {
        crate::video::wait_video(self, VideoPoll::default()).await
    }
}
