//! Wires `*Text.stream` against the legacy `prompt_stream` callback API.
//!
//! Rust's stream surface is callback-based by design — the same
//! "trailing-handle" shape used in Go (`*TextStream`), TS
//! (`TextStream`) and Python (`TextStream`), expressed differently:
//!
//! - Chunks → the user-supplied `callback` (a `FnMut(&str)`) is invoked
//!   for each delta as it arrives. Equivalent to iterating the
//!   `AsyncIterable<string>` / `iter.Seq2[string, error]` in the other
//!   SDKs.
//! - Trailing handle → the function returns `Result<Response>` with
//!   the accumulated text, token counts, and any terminal error.
//!   Equivalent to `stream.response()` / `stream.Response()` after
//!   iteration completes in the other SDKs.
//!
//! The `impl Stream<Item = …>` variant from the `futures` crate would
//! mirror the other SDKs visually, but it requires a third-party
//! dependency (`futures-core` at minimum) which the project's
//! stdlib-only rule does not permit. The callback shape is functionally
//! equivalent and stays dependency-free.

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
