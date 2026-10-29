//! Part-kind pre-flight check (ADR-100 MSG-011). Mirror of go/part_kinds.go.
//!
//! The `Part` enum makes an empty or two-kind part unrepresentable, so the
//! only rule left is the capability's accepted kinds, read from the
//! generated `ACCEPTED_PART_KINDS` table.

use crate::error::Error;
use crate::structs::{Part, ACCEPTED_PART_KINDS};

/// Reject, before any HTTP call, a part whose kind the capability does not
/// accept, naming the kind. `capability` is the capability label that keys
/// the generated table, for example `"image_generation"`.
pub(crate) fn check_part_kinds(capability: &str, parts: &[Part]) -> Result<(), Error> {
    let accepted = ACCEPTED_PART_KINDS
        .iter()
        .find(|(cap, _)| *cap == capability)
        .map(|(_, kinds)| *kinds)
        .unwrap_or(&[]);
    for part in parts {
        let kind = part.kind();
        if !accepted.contains(&kind) {
            return Err(Error::Validation {
                field: "parts",
                message: format!(
                    "{} does not accept {kind} parts",
                    capability.replace('_', " ")
                ),
            });
        }
    }
    Ok(())
}















































































