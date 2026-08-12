//! Verbatim assistant-turn capture (ADR-085).
//!
//! llmkit keeps a canonical projection of every turn — role, content, tool
//! calls — and used to rebuild the next request's assistant turn from it. That
//! projection is lossy in three ways (ADR-085 §1): it has nowhere to put
//! reasoning, it drops assistant prose that accompanies a tool call, and it has
//! no slot for per-part provider metadata. The fix is not a richer projection
//! but a second representation alongside it: keep the provider's own bytes for
//! the turn and send those back unchanged.
//!
//! Nothing here parses the payload. The only structure this file reads is the
//! path down to the turn — everything at and below it is carried as the
//! provider wrote it.
//!
//! Rust-specific note. Capture holds the provider's bytes through
//! `serde_json::value::RawValue`, which keeps the source slice of every value
//! it descends through. `serde_json::Value` cannot: its default `Map` is a
//! `BTreeMap`, so decoding and re-encoding a turn re-sorts every object key.
//! That is why [`crate::structs::ProviderTurn::wire`] is a `String` and why the
//! walk below never touches `Value`. On the SEND leg the splice does parse the
//! string into a `Value`, because that is the type the request body is
//! assembled in — so Rust transmits semantically identical JSON with sorted
//! keys, which is what ADR-085 RSN-002's amendment permits (verbatim at REST is
//! the guarantee; byte-identical transmission is a Go-only property).

use std::collections::HashMap;

use serde_json::value::RawValue;

use crate::providers::generated::providers::ProviderSpec;
use crate::structs::ProviderTurn;
use crate::transforms::Msg;

/// Splits one dot-notation path segment into a field name and an array index:
/// `"choices[0]"` -> `("choices", Some(0))`, `"message"` -> `("message", None)`.
///
/// single parser here, so capture cannot drift from the path facts it reads.
///
/// A malformed segment yields the whole segment as a field name and no index.
/// That resolves to "no such field" one line later, which is the same outcome
/// as a path that does not exist — never a silently widened match to the whole
/// array, which is what a negative index would read back as.
pub(crate) fn split_path_segment(part: &str) -> (&str, Option<usize>) {
    let Some(bracket) = part.find('[') else {
        return (part, None);
    };
    if !part.ends_with(']') {
        return (part, None);
    }
    match part[bracket + 1..part.len() - 1].parse::<usize>() {
        Ok(index) => (&part[..bracket], Some(index)),
        Err(_) => (part, None),
    }
}

/// Returns the VERBATIM JSON text of the value at `path`, or `None` when the
/// path does not resolve.
///
/// The distinction from `paths::navigate_path` is the whole point: that walker
/// descends a `Value` that has already been parsed, so re-encoding its result
/// emits serde's rendering of the value (object keys sorted, numbers
/// reformatted) rather than the provider's. `RawValue` keeps the source slice
/// of each value it decodes, so descending through it preserves the bytes.
pub(crate) fn extract_raw_json_path(body: &str, path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let mut current: Box<RawValue> = serde_json::from_str(body).ok()?;
    for part in path.split('.') {
        let (field, index) = split_path_segment(part);
        if !field.is_empty() {
            let mut object: HashMap<String, Box<RawValue>> =
                serde_json::from_str(current.get()).ok()?;
            current = object.remove(field)?;
        }
        if let Some(index) = index {
            let array: Vec<Box<RawValue>> = serde_json::from_str(current.get()).ok()?;
            current = array.into_iter().nth(index)?;
        }
    }
    Some(current.get().to_string())
}

/// Returns where one replayable assistant turn sits in a response body for this
/// provider under this wire shape, or `""` when the shape declares no position.
///
/// An empty result is a DECLARED absence, not a missing lookup: `ChatBedrock`
/// carries `assistantTurnUnanchored` rather than a path, because nobody has
/// probed what an assistant turn looks like on Converse (ADR-085 OQ-5). The
///
/// "declared unanchored" and never "somebody forgot".
pub(crate) fn assistant_turn_path(config: &ProviderSpec, chat_wire_shape: &str) -> &'static str {
    config
        .chat_protocols
        .iter()
        .find(|protocol| protocol.wire_shape == chat_wire_shape)
        .map_or("", |protocol| protocol.assistant_turn_path)
}

/// Resolves the shape a response was produced under. An empty argument means
/// the caller did not route through `.protocol(...)` — the batch path passes
/// `""` because batch is Chat-Completions-only (ADR-055) — so the provider's
/// default shape applies.
fn effective_chat_wire_shape<'a>(config: &'a ProviderSpec, chat_wire_shape: &'a str) -> &'a str {
    if chat_wire_shape.is_empty() {
        config.chat_wire_shape
    } else {
        chat_wire_shape
    }
}

/// Lifts the assistant turn out of a response body, or returns `None` when this
/// shape declares no turn position or the body carries nothing there.
pub(crate) fn capture_provider_turn(
    body: &str,
    config: &ProviderSpec,
    chat_wire_shape: &str,
) -> Option<ProviderTurn> {
    let shape = effective_chat_wire_shape(config, chat_wire_shape);
    let wire = extract_raw_json_path(body, assistant_turn_path(config, shape))?;
    // A JSON null at the path is the provider declining to send a turn, not a
    // turn whose content is null — Google nulls candidates[0].content on a
    // safety block, and OpenAI-compatible proxies null choices[0].message on a
    // content filter. `RawValue` holds those four bytes like any other value,
    // so an emptiness check alone captures it and the next request appends a
    // bare `null` to the message array, which is a 400.
    if wire.trim().is_empty() || wire.trim() == "null" {
        return None;
    }
    Some(ProviderTurn {
        wire_shape: shape.to_string(),
        wire,
    })
}

/// The RSN-006 boundary: a captured payload is replayed only under the shape
/// that produced it, and a mismatch drops it and reconstructs the turn from the
/// canonical projection instead.
///
/// One unconditional rule, applied once per request where the config and the
/// message list first meet, so no transform has to remember the check. The
/// draft ADR made this branch on whether the provider mandates the echo and
/// raised an error on the mandating ones; RESEARCH-017 measured that set to be
/// empty, so only the drop arm was ever reachable.
///
/// Dropping is the safe direction here, and the measurement is why: every
/// probed provider ACCEPTS a request with the payload omitted, while a mangled
/// payload is the single 400 anywhere in the matrix. Replaying an Anthropic
/// block array into Google's contents array would be exactly that mangling.
pub(crate) fn resolve_turns(msgs: &[Msg], config: &ProviderSpec) -> Vec<Msg> {
    msgs.iter()
        .map(|m| match m {
            // Two conditions, not one. Matching the shape is not enough: the
            // shape must also DECLARE a turn position. A payload claiming an
            // unanchored shape can only come from caller-supplied or loaded
            // data, and the transform for such a shape has no replay arm — so
            // without the second check, a history carrying
            // ProviderTurn{wire_shape: "ChatBedrock"} reaches a builder with
            // nowhere to put it.
            Msg::Turn { shape, fallback, .. }
                if shape != config.chat_wire_shape
                    || assistant_turn_path(config, shape).is_empty() =>
            {
                (**fallback).clone()
            }
            other => other.clone(),
        })
        .collect()
}






















































































































































































































































































































































































































































































































































