// Public provider namespace (ADR-038, kept public by the owner as an exception
// to ADR-103 PUB-001): the provider names and the keyless roster,
// providers::info(name) / providers::list(). Keyless because the headline use
// case ("which env var holds the key?") comes before a client exists.
//
// The generated tables are crate-internal (ADR-103). Every SDK emits the same
// facts and the Rust runtime reads a subset, so an unread entry is not dead
//
// (CachingMode::AutomaticCaching), so clippy's variant-postfix rule, which
// applies once a type is not exported, does not apply either.
#[allow(dead_code, clippy::enum_variant_names)]
pub(crate) mod generated;

pub use generated::provider_info::{info, list, ProviderInfo};
pub use generated::providers::{ProviderName, UnknownProviderError};



















































































