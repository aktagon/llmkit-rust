//! Request-variant selection — mirror of go/request_variant.go (ADR-096).

use crate::error::Error;
use crate::providers::generated::image_gen::{RequestVariant, TEXT_ONLY, WITH_REFERENCE_IMAGES};
use crate::providers::generated::providers::ProviderName;

/// Picks the config's wire request by the inputs the request carries
/// (ADR-096): `WithReferenceImages` when it holds at least one image part,
/// else `TextOnly`. A config with no variant for those inputs cannot send
/// the request, so the caller gets a validation error on "parts".
pub(crate) fn select_request_variant(
    provider: ProviderName,
    variants: &'static [RequestVariant],
    image_count: usize,
) -> Result<&'static RequestVariant, Error> {
    let carries = if image_count > 0 {
        WITH_REFERENCE_IMAGES
    } else {
        TEXT_ONLY
    };
    variants
        .iter()
        .find(|v| v.carries == carries)
        .ok_or_else(|| Error::Validation {
            field: "parts",
            message: format!("{provider} has no {carries} request"),
        })
}

































