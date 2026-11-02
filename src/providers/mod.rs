// Public provider namespace (ADR-038, kept public by the owner as an exception
// to ADR-104 PUB-001): the provider names and the keyless roster,
// providers::info(name) / providers::list(). Keyless because the headline use
// case ("which env var holds the key?") comes before a client exists.
//
// The generated tables are crate-internal (ADR-104). Every SDK emits the same
// facts and the Rust runtime reads a subset, so an unread entry is not dead
//
// (CachingMode::AutomaticCaching), so clippy's variant-postfix rule, which
// applies once a type is not exported, does not apply either.
#[allow(dead_code, clippy::enum_variant_names)]
pub(crate) mod generated;

// The keyless roster functions (ADR-038). Their types, ProviderName,
// ProviderInfo and UnknownProviderError, are re-exported from the crate root
// only, so each roster name has one path (ADR-104 PUB-009).
pub use generated::provider_info::{info, list};

// The keyless read of the models this SDK version states facts for (ADR-102
// MPT-010), beside the roster. Its one public name is declared here.
pub use crate::models::known_models;



















































































