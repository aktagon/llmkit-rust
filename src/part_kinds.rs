//! Part-kind pre-flight check (ADR-100 MSG-011). Mirror of go/part_kinds.go.
//!
//! The `Part` enum makes a part with no kind or two unrepresentable, so the
//! rules left are an empty string part ([`check_part`]) and the capability's
//! accepted kinds, read from the generated `ACCEPTED_PART_KINDS` table.

use crate::error::Error;
use crate::structs::{Part, ACCEPTED_PART_KINDS};

/// Reject, before any HTTP call, an empty text, lyrics or audio_url part
/// ([`check_part`]) and a part whose kind the capability does not accept,
/// naming the kind, on field `parts[i]` as Go and TS do. Chat and every other capability run it.
/// `capability` is the capability label that keys the generated table, for
/// example `"image_generation"`.
pub(crate) fn check_part_kinds(capability: &str, parts: &[Part]) -> Result<(), Error> {
    let accepted = ACCEPTED_PART_KINDS
        .iter()
        .find(|(cap, _)| *cap == capability)
        .map(|(_, kinds)| *kinds)
        .unwrap_or(&[]);
    for (i, part) in parts.iter().enumerate() {
        check_part(part).map_err(|message| Error::Validation {
            field: format!("parts[{i}]"),
            message,
        })?;
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

/// The one check of a part's shape, run by [`check_part_kinds`] on every
/// capability and by `save_history`, so a request and a Save reject the same
/// part with the same error (HANDOFF-105 F11, F12): a string variant that is
/// empty, which sets no kind in Go, where the empty string is unset (owner
/// decision 2; HANDOFF-111 for lyrics and audio_url). The `Part` enum leaves
/// no other shape to check. The match names every variant, so a new string
/// variant fails to compile until it is placed here. The error is the
/// message; the caller names the field.
pub(crate) fn check_part(part: &Part) -> Result<(), String> {
    match part {
        Part::Text(s) | Part::Lyrics(s) | Part::AudioUrl(s) if s.is_empty() => {
            Err("empty part".into())
        }
        Part::Text(_)
        | Part::Lyrics(_)
        | Part::AudioUrl(_)
        | Part::Image(_)
        | Part::File(_)
        | Part::ToolCall(_)
        | Part::ToolResult(_)
        | Part::Audio(_) => Ok(()),
    }
}















































































































