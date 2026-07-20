//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::future::{Future, IntoFuture};
use std::pin::Pin;

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

    submit_video(&provider, &request, &b.middleware, b.raw).await
}

///
///
///
///
#
pub trait VideoHandleExt {
    async fn wait(&self) -> Result<VideoResponse, Error>;
}

impl VideoHandleExt for VideoHandle {
    async fn wait(&self) -> Result<VideoResponse, Error> {
        crate::video::wait_video(self, VideoPoll::default()).await
    }
}

//
//
impl IntoFuture for VideoHandle {
    type Output = Result<VideoResponse, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.wait().await })
    }
}
