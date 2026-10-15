//! Request-variant selection — mirror of go/request_variant.go (ADR-096).

use crate::error::Error;
use crate::providers::generated::image_gen::{RequestVariant, TEXT_ONLY};
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
            field: "parts",
            message: format!("{provider} has no {} request", carries.join("+")),
        })
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









































































