// Code generated — DO NOT EDIT.

//! Catalogue builder types (ADR-019). Chain methods take `mut self`
//! and return `Self` for chain immutability via ownership transfer.
//! HTTP-touching terminals delegate to hand-coded runtime in
//! `crate::models`.

use super::Client;
use crate::models::{
    catalogue_providers_list, catalogue_run_get, catalogue_run_list, catalogue_run_live,
    CatalogueError,
};
use crate::providers::generated::provider_info::ProviderInfo;
use crate::structs::{LiveResult, ModelInfo};
use crate::types::{Capability, Provider};

/// Models is the catalogue builder. Chain methods consume `self` and
/// return a fresh `Models`; `list().await` fans out HTTP across
/// configured providers, served from the client's one-hour model-list
/// cache unless `fresh()` is set; `provider(p)` returns `ScopedModels`.
#[derive(Clone, Debug)]
pub struct Models {
    pub client: Client,
    pub cap_filter: Option<Capability>,
    pub fresh_flag: bool,
}

impl Models {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            cap_filter: None,
            fresh_flag: false,
        }
    }

    ///
    /// slice contains `c`. Composes with `list` / `provider(p).list`.
    pub fn with_capability(mut self, c: Capability) -> Self {
        self.cap_filter = Some(c);
        self
    }

    /// Bypass the client's model-list cache: the live call is made, and a
    /// successful list refills the cache. Carries through `provider(p)`.
    pub fn fresh(mut self) -> Self {
        self.fresh_flag = true;
        self
    }

    /// Scope the catalogue to a single provider; returns `ScopedModels`
    /// on which `raw()`, `list().await`, and `get(id).await` are reachable.
    pub fn provider(&self, p: Provider) -> ScopedModels {
        ScopedModels {
            client: self.client.clone(),
            target: p,
            cap_filter: self.cap_filter,
            raw_flag: false,
            fresh_flag: self.fresh_flag,
        }
    }

    /// The live model list across every provider this `Client` is
    /// credentialed for, as an aggregated `LiveResult`. Today a `Client`
    /// carries one provider, so the result is 0 or 1 underlying calls;
    /// the API shape leaves room for a future multi-credential `Client`
    /// without breaking callers. `with_capability` composes post-fetch.
    pub async fn list(&self) -> LiveResult {
        catalogue_run_live(self).await
    }
}

/// ScopedModels is the single-provider live-catalogue sub-builder.
/// Reached via `Models::provider(p)`. `raw()` opts into populating
/// `ModelInfo.raw` per ADR-014.
#[derive(Clone, Debug)]
pub struct ScopedModels {
    pub client: Client,
    pub target: Provider,
    pub cap_filter: Option<Capability>,
    pub raw_flag: bool,
    pub fresh_flag: bool,
}

impl ScopedModels {
    pub fn raw(mut self) -> Self {
        self.raw_flag = true;
        self
    }

    /// This provider's model list: from the client's cache when it holds
    /// a list younger than one hour, otherwise from the live call.
    pub async fn list(&self) -> Result<Vec<ModelInfo>, CatalogueError> {
        catalogue_run_list(self).await
    }

    /// One model record by id: from the cached list when it is fresh and
    /// holds the id, otherwise from the live point lookup.
    pub async fn get(&self, id: &str) -> Result<ModelInfo, CatalogueError> {
        catalogue_run_get(self, id).await
    }
}

/// Providers is the providers-namespace prototype. `list()` returns the
/// providers with both credentials configured and `hasModelsEndpoint`
/// declared, as secret-free `ProviderInfo` (ADR-040 PSR-005). The static
/// roster of every supported provider is `providers::list()`.
#[derive(Clone, Debug)]
pub struct Providers {
    pub client: Client,
}

impl Providers {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    pub fn list(&self) -> Vec<&'static ProviderInfo> {
        catalogue_providers_list(&self.client)
    }
}
