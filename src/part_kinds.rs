//! Part-kind pre-flight check (ADR-100 MSG-011). Mirror of go/part_kinds.go.
//!
//! The `Part` enum makes an empty or two-kind part unrepresentable, so the
//! only rule left is the capability's accepted kinds, read from the
//! generated `ACCEPTED_PART_KINDS` table.

use crate::error::Error;
use crate::structs::{Part, ACCEPTED_PART_KINDS};

/// Reject, before any HTTP call, a part whose kind the capability does not
/// accept, naming the kind, on field `parts[i]` as Go and TS do.
/// `capability` is the capability label that keys the generated table, for
/// example `"image_generation"`.
pub(crate) fn check_part_kinds(capability: &str, parts: &[Part]) -> Result<(), Error> {
    let accepted = ACCEPTED_PART_KINDS
        .iter()
        .find(|(cap, _)| *cap == capability)
        .map(|(_, kinds)| *kinds)
        .unwrap_or(&[]);
    for (i, part) in parts.iter().enumerate() {
        let kind = part.kind();
        if !accepted.contains(&kind) {
            return Err(Error::Validation {
                field: format!("parts[{i}]"),
                message: format!(
                    "{} does not accept {kind} parts",
                    capability.replace('_', " ")
                ),
            });
        }
    }
    Ok(())
}

/// The one check of a chat part's shape, run by the request side
/// ([`crate::transforms::check_turn_parts`]) and by `save_history`, so both
/// reject the same part with the same error (HANDOFF-105 F11): a text part
/// with no text, which sets no kind in Go (owner decision 2). The `Part` enum
/// leaves no other shape to check. The error is the message; the caller names
/// the field.
pub(crate) fn check_chat_part(part: &Part) -> Result<(), String> {
    match part {
        Part::Text(text) if text.is_empty() => Err("empty part".into()),
        _ => Ok(()),
    }
}
















































































