//! Phase 3 slice 2b — wires Text::stream against the legacy
//! `prompt_stream` callback API.
//!
//! Stream signature: `stream(self, msg, callback) -> Result<Response>`
//! where `callback: impl FnMut(&str)`. This matches the legacy
//! callback-based shape rather than `impl Stream` from the `futures`
//! crate — picking that variant would add a runtime dependency, and
//! the callback form covers the existing use case.
//!
//! Future plan-016 follow-up may add a futures::Stream variant once
//! we add a dependency choice (futures-core is small but still a dep).

use crate::error::Error;
use crate::types::Response;

use super::text::{build_options, build_provider, build_request};
use super::Text;

pub async fn text_stream(
    b: Text,
    msg: impl Into<String>,
    callback: impl FnMut(&str),
) -> Result<Response, Error> {
    let final_text: String = msg.into();
    let provider = build_provider(&b);
    let request = build_request(&b, &final_text);
    let options = build_options(&b);
    crate::prompt_stream_internal(&provider, &request, options, callback).await
}
