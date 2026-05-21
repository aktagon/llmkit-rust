//! Hand-coded catalogue runtime (ADR-019). The generated builder types
//! in `builders/catalogue.rs` delegate their terminal methods here.
//!
//! Folds in the providers-namespace runtime (`catalogue_providers_*`)
//! because `crate::providers` is the generated subpackage path and Rust
//! forbids shadowing it with a sibling module.

use crate::builders::Client;
use crate::builders::catalogue::{Models, ScopedModels};
use crate::catalogue::{catalogue_config, COMPILED_IN_MODELS};
use crate::providers::generated::providers::{ProviderName, ALL_PROVIDER_NAMES};
use crate::structs::{LiveResult, ModelInfo, ProviderError};
use crate::types::{Capability, Provider};

/// Catalogue error sentinels (ADR-019). Live provider calls map to one
/// of these variants:
///
/// * [`CatalogueError::NotSupported`] — provider lacks
///   `llm:hasModelsEndpoint` (no `/v1/models` route; nothing to fetch).
///   Vertex and Bedrock surface this until their dedicated parsers land.
/// * [`CatalogueError::Scope`] — HTTP 403 whose body mentions scope
///   (OpenAI's `api.model.read` scope is the canonical case).
/// * [`CatalogueError::Unavailable`] — any other non-2xx response or
///   network failure during a live HTTP call.
#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("llmkit: provider does not expose a models endpoint")]
    NotSupported,
    #[error("llmkit: provider models endpoint unavailable")]
    Unavailable,
    #[error("llmkit: api key lacks scope for models endpoint")]
    Scope,
}

impl CatalogueError {
    /// Wire-format discriminant carried in [`ProviderError::kind`] (ADR-019
    /// Amendment 1). Lets consumers branch typed across all four SDKs via
    /// a single string compare.
    pub fn kind(&self) -> &'static str {
        match self {
            CatalogueError::NotSupported => "not_supported",
            CatalogueError::Unavailable => "unavailable",
            CatalogueError::Scope => "scope",
        }
    }
}

/// Walk the compiled-in slice and return owned `ModelInfo` records
/// matching the optional capability filter.
pub(crate) fn catalogue_filter(cap_filter: Option<Capability>) -> Vec<ModelInfo> {
    COMPILED_IN_MODELS
        .iter()
        .filter(|m| match cap_filter {
            None => true,
            Some(c) => m.capabilities.contains(&c),
        })
        .map(compiled_to_model_info)
        .collect()
}

/// Linear scan over the compiled-in slice. Returns `None` on miss.
pub(crate) fn catalogue_lookup(id: &str) -> Option<ModelInfo> {
    COMPILED_IN_MODELS
        .iter()
        .find(|m| m.id == id)
        .map(compiled_to_model_info)
}

/// Aggregate live results across configured providers. Phase 3 wires
/// real HTTP; this scaffold inherits the same partial-success shape so
/// the builder surface is stable. `with_capability` composes post-fetch.
pub(crate) async fn catalogue_run_live(models: &Models) -> LiveResult {
    use std::collections::HashMap;
    let configured = models.client.providers().list();
    let mut all: Vec<ModelInfo> = Vec::new();
    let mut errors: HashMap<String, ProviderError> = HashMap::new();
    for p in configured {
        let scoped = ScopedModels {
            client: models.client.clone(),
            target: p.clone(),
            cap_filter: models.cap_filter,
            raw_flag: false,
        };
        match scoped.list().await {
            Ok(models) => all.extend(models),
            Err(err) => {
                // ADR-019 Amendment 1: structured discriminant + message.
                errors.insert(
                    provider_name_slug(p.name).to_string(),
                    ProviderError { kind: err.kind().to_string(), message: err.to_string() },
                );
            }
        }
    }
    if let Some(c) = models.cap_filter {
        all.retain(|m| m.capabilities.contains(&c));
    }
    all.sort_by(|a, b| {
        // Comparator runs O(n log n) times — keep it allocation-free.
        let pa = provider_name_slug(a.provider.name);
        let pb = provider_name_slug(b.provider.name);
        pa.cmp(pb).then_with(|| a.id.cmp(&b.id))
    });
    LiveResult { models: all, errors }
}

/// Single-provider live HTTP — Phase 3 stub.
///
/// Kept `async` so the public signature is stable when Phase 3 wires
/// the real HTTP call. `unused_async` allow is the canonical breadcrumb
/// for "this will await soon."
#[allow(clippy::unused_async)] // Phase 3 will introduce .await on post_json
pub(crate) async fn catalogue_run_list(scoped: &ScopedModels) -> Result<Vec<ModelInfo>, CatalogueError> {
    if catalogue_config(scoped.target.name).is_none() {
        return Err(CatalogueError::NotSupported);
    }
    Err(CatalogueError::Unavailable)
}

/// Single-provider live model fetch — Phase 3 stub.
#[allow(clippy::unused_async)] // Phase 3 will introduce .await on get_json
pub(crate) async fn catalogue_run_get(
    scoped: &ScopedModels,
    _id: &str,
) -> Result<ModelInfo, CatalogueError> {
    if catalogue_config(scoped.target.name).is_none() {
        return Err(CatalogueError::NotSupported);
    }
    Err(CatalogueError::Unavailable)
}

// === Providers-namespace runtime (hand-coded mirror of go/providers.go) ===

/// Eligibility test per ADR-019: credentials configured on this `Client`
/// AND `llm:hasModelsEndpoint` declared in the ontology. A Rust `Client`
/// carries one provider's credentials, so the result is either a
/// single-element vec (when its provider has a catalogue endpoint) or
/// empty.
pub(crate) fn catalogue_providers_list(client: &Client) -> Vec<Provider> {
    if catalogue_config(client.provider.name).is_none() {
        return Vec::new();
    }
    vec![Provider {
        name: client.provider.name,
        api_key: client.provider.api_key.clone(),
        model: None,
        base_url: client.provider.base_url.clone(),
    }]
}

/// Every provider the SDK was built to support — independent of `Client`
/// credentials. Sorted by wire-format name for deterministic callers.
pub(crate) fn catalogue_providers_supported() -> Vec<Provider> {
    // Pair each name with its &'static slug so sort can compare without
    // allocating; the slug is dropped after sort.
    let mut named: Vec<(ProviderName, &'static str)> = ALL_PROVIDER_NAMES
        .iter()
        .map(|n| (*n, provider_name_slug(*n)))
        .collect();
    named.sort_by_key(|(_, slug)| *slug);
    named
        .into_iter()
        .map(|(n, _)| Provider {
            name: n,
            api_key: String::new(),
            model: None,
            base_url: None,
        })
        .collect()
}

// === Internal helpers ===

fn compiled_to_model_info(def: &crate::catalogue::CompiledModelDef) -> ModelInfo {
    ModelInfo {
        id: def.id.to_string(),
        provider: Provider {
            name: def.provider,
            api_key: String::new(),
            model: None,
            base_url: None,
        },
        capabilities: def.capabilities.to_vec(),
        display_name: def.display_name.to_string(),
        description: def.description.to_string(),
        context_window: def.context_window,
        max_output: def.max_output,
        created: 0,
        raw: None,
    }
}

/// Convert `ProviderName` to its wire-format slug ("anthropic", "openai", ...).
/// The generated `ProviderConfig.slug` is `&'static str`, so we hand back the
/// borrow directly — no heap allocation. Callers that need ownership call
/// `.to_string()` themselves (see the `errors.insert` site below).
fn provider_name_slug(name: ProviderName) -> &'static str {
    crate::providers::generated::providers::provider_config(name).slug
}
