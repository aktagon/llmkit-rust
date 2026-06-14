pub mod generated;

// Public provider namespace (ADR-038): the narrow per-provider catalogue,
// keyless (no client needed — the headline use case is "which env var holds the
// key?", asked before a client exists): providers::info(name) /
// providers::list(). The internal wire/transform spec stays in `generated` and
// is not surfaced here.
pub use generated::provider_info::{info, list, ProviderInfo};
