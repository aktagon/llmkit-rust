//! Wires `*Text.stream` to [`TextStream`], the trailing handle every SDK
//! returns: pull each delta with `next().await`, then read `response()`.

use crate::stream::TextStream;

use super::text::{build_options, build_protocol, build_provider, build_request};
use super::Text;

pub(crate) fn text_stream(b: Text, msg: impl Into<String>) -> TextStream {
    // ADR-055: the stream path resolves a non-default protocol's (Responses)
    // endpoint, body and stream facts (HANDOFF-087).
    let final_text: String = msg.into();
    TextStream::new(
        build_provider(&b),
        build_request(&b, &final_text),
        build_protocol(&b).to_string(),
        build_options(&b),
    )
}
