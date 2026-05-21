//!
//!
//!
//!
//!
//!

use crate::builders::Client;
use crate::builders::catalogue::{Models, ScopedModels};
use crate::catalogue::{catalogue_config, COMPILED_IN_MODELS};
use crate::providers::generated::providers::{ProviderName, ALL_PROVIDER_NAMES};
use crate::structs::{LiveResult, ModelInfo};
use crate::types::{Capability, Provider};

///
///
///
///
///
///
///
///
///
///
#[derive(Debug, thiserror::Error)]
pub enum CatalogueError {
    #[error("llmkit: provider does not expose a models endpoint")]
    NotSupported,
    #[error("llmkit: provider models endpoint unavailable")]
    Unavailable,
    #[error("llmkit: api key lacks scope for models endpoint")]
    Scope,
}

///
///
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

///
pub(crate) fn catalogue_lookup(id: &str) -> Option<ModelInfo> {
    COMPILED_IN_MODELS
        .iter()
        .find(|m| m.id == id)
        .map(compiled_to_model_info)
}

///
///
///
pub(crate) async fn catalogue_run_live(models: &Models) -> LiveResult {
    use std::collections::HashMap;
    let configured = models.client.providers().list();
    let mut all: Vec<ModelInfo> = Vec::new();
    let mut errors: HashMap<String, String> = HashMap::new();
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
                //
                errors.insert(provider_name_slug(p.name).to_string(), err.to_string());
            }
        }
    }
    if let Some(c) = models.cap_filter {
        all.retain(|m| m.capabilities.contains(&c));
    }
    all.sort_by(|a, b| {
        //
        let pa = provider_name_slug(a.provider.name);
        let pb = provider_name_slug(b.provider.name);
        pa.cmp(pb).then_with(|| a.id.cmp(&b.id))
    });
    LiveResult { models: all, errors }
}

///
///
///
///
///
#[allow(clippy::unused_async)] // Phase 3 will introduce .await on post_json
pub(crate) async fn catalogue_run_list(scoped: &ScopedModels) -> Result<Vec<ModelInfo>, CatalogueError> {
    if catalogue_config(scoped.target.name).is_none() {
        return Err(CatalogueError::NotSupported);
    }
    Err(CatalogueError::Unavailable)
}

///
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

//

///
///
///
///
///
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

///
///
pub(crate) fn catalogue_providers_supported() -> Vec<Provider> {
    //
    //
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

//

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

///
///
///
///
fn provider_name_slug(name: ProviderName) -> &'static str {
    crate::providers::generated::providers::provider_config(name).slug
}
