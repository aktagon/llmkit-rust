// Code generated — DO NOT EDIT.


//!
//!
//!

use crate::providers::generated::providers::ProviderName;
use crate::types::Capability;

///
///
///
#[derive(Clone, Copy, Debug)]
pub struct CompiledModelDef {
    pub id: &'static str,
    pub provider: ProviderName,
    pub capabilities: &'static [Capability],
    pub display_name: &'static str,
    pub description: &'static str,
    pub context_window: i64,
    pub max_output: i64,
}

pub static COMPILED_IN_MODELS: &[CompiledModelDef] = &[
];

///
///
///
///
#[allow(dead_code)] // Phase 3: consumed by parser dispatch in catalogue_run_list
pub(crate) fn ontology_capabilities(
    provider: ProviderName,
    model_id: &str,
) -> Option<&'static [Capability]> {
    for m in COMPILED_IN_MODELS {
        if m.provider == provider && m.id == model_id {
            return Some(m.capabilities);
        }
    }
    None
}

///
///
///
#[derive(Clone, Copy, Debug)]
pub struct CatalogueConfig {
    pub endpoint: &'static str,
    pub pagination: &'static str,
    pub spec_url: &'static str,
    pub spec_format: &'static str,
}

static ANTHROPIC_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "CursorByLastID",
    spec_url: "https://github.com/anthropics/anthropic-sdk-typescript/blob/main/api.md",
    spec_format: "OpenAPI3",
};

static GOOGLE_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1beta/models",
    pagination: "CursorOpaqueToken",
    spec_url: "https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta",
    spec_format: "GoogleDiscovery",
};

static OPENAI_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    spec_url: "https://github.com/openai/openai-openapi/blob/master/openapi.yaml",
    spec_format: "OpenAPI3",
};

pub(crate) fn catalogue_config(provider: ProviderName) -> Option<&'static CatalogueConfig> {
    match provider {
        ProviderName::Anthropic => Some(&ANTHROPIC_CATALOGUE),
        ProviderName::Google => Some(&GOOGLE_CATALOGUE),
        ProviderName::OpenAI => Some(&OPENAI_CATALOGUE),
        _ => None,
    }
}
