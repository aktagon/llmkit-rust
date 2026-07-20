//

//!
//!
//!
//!

use super::{Client};
use crate::models::{
    catalogue_filter, catalogue_lookup, catalogue_providers_list,
    catalogue_run_get, catalogue_run_list, catalogue_run_live, CatalogueError,
};
use crate::providers::generated::provider_info::ProviderInfo;
use crate::structs::{LiveResult, ModelInfo};
use crate::types::{Capability, Provider};

///
///
///
///
#
pub struct Models {
    pub client: Client,
    pub cap_filter: Option<Capability>,
}

impl Models {
    pub(crate) fn new(client: Client) -> Self {
        Self { client, cap_filter: None }
    }

    ///
    ///
    pub fn with_capability(mut self, c: Capability) -> Self {
        self.cap_filter = Some(c);
        self
    }

    ///
    ///
    pub fn provider(&self, p: Provider) -> ScopedModels {
        ScopedModels {
            client: self.client.clone(),
            target: p,
            cap_filter: self.cap_filter,
            raw_flag: false,
        }
    }

    ///
    ///
    pub fn list(&self) -> Vec<ModelInfo> {
        catalogue_filter(self.cap_filter)
    }

    ///
    pub fn get(&self, id: &str) -> Option<ModelInfo> {
        catalogue_lookup(id)
    }

    ///
    ///
    ///
    ///
    ///
    pub async fn live(&self) -> LiveResult {
        catalogue_run_live(self).await
    }
}

///
///
///
#
pub struct ScopedModels {
    pub client: Client,
    pub target: Provider,
    pub cap_filter: Option<Capability>,
    pub raw_flag: bool,
}

impl ScopedModels {
    pub fn raw(mut self) -> Self {
        self.raw_flag = true;
        self
    }

    pub async fn list(&self) -> Result<Vec<ModelInfo>, CatalogueError> {
        catalogue_run_list(self).await
    }

    pub async fn get(&self, id: &str) -> Result<ModelInfo, CatalogueError> {
        catalogue_run_get(self, id).await
    }
}

///
///
///
///
#
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
