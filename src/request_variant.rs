//! Request-variant selection — mirror of go/request_variant.go (ADR-096).

use crate::error::Error;
use crate::providers::generated::image_gen::{
    RequestVariant, TEXT_ONLY, WITH_LAST_FRAME, WITH_REFERENCE_IMAGES, WITH_SEED_FRAME,
};
use crate::providers::generated::providers::ProviderName;

/// Picks the config's wire request whose carried inputs equal `carries`, in
/// any order (ADR-096). A first-and-last-frame request carries two inputs. A
/// config with no variant for those inputs cannot send the request, so the
/// caller gets a validation error on "parts".
pub(crate) fn select_request_variant(
    provider: ProviderName,
    variants: &'static [RequestVariant],
    carries: &[&'static str],
) -> Result<&'static RequestVariant, Error> {
    variants
        .iter()
        .find(|v| v.carries.len() == carries.len() && v.carries.iter().all(|c| carries.contains(c)))
        .ok_or_else(|| Error::Validation {
            field: "parts".into(),
            message: format!("{provider} has no {} request", carried_input_words(carries)),
        })
}

/// Names carried inputs in the words an error message uses, such as "seed
/// frame and last frame"; the constant names are internal (ADR-103 PUB-005).
/// An input with no entry keeps its constant name.
fn carried_input_words(carries: &[&'static str]) -> String {
    carries
        .iter()
        .map(|&c| match c {
            TEXT_ONLY => "text-only",
            WITH_REFERENCE_IMAGES => "reference image",
            WITH_SEED_FRAME => "seed frame",
            WITH_LAST_FRAME => "last frame",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

/// The carried input of a request with `image_count` image parts:
/// `image_input` when it holds at least one, else `TextOnly`. The capability
/// names what its image parts are: `WithReferenceImages` for image,
/// `WithSeedFrame` for video.
pub(crate) fn parts_carry(image_count: usize, image_input: &'static str) -> Vec<&'static str> {
    if image_count > 0 {
        vec![image_input]
    } else {
        vec![TEXT_ONLY]
    }
}









































































