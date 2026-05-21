//! Hand-coded catalogue runtime (ADR-019). The generated builder types
//! in `builders/catalogue.rs` delegate their terminal methods here.
//!
//! Folds in the providers-namespace runtime (`catalogue_providers_*`)
//! because `crate::providers` is the generated subpackage path and Rust
//! forbids shadowing it with a sibling module.

use crate::builders::Client;
use crate::builders::catalogue::{Models, ScopedModels};
use crate::catalogue::{
    catalogue_config, ontology_capabilities, CatalogueConfig, COMPILED_IN_MODELS,
};
use crate::http::get_text;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::providers::{
    provider_config, ProviderConfig, ProviderName, ALL_PROVIDER_NAMES,
};
use crate::providers::generated::models_parsers::{
    parse_anthropic_models_response, parse_google_models_response,
    parse_openai_cohort_models_response, ParsedModelRecord, ParsedModelsPage,
};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
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
    #[error("llmkit: provider models endpoint unavailable: {0}")]
    Unavailable(String),
    #[error("llmkit: api key lacks scope for models endpoint: {0}")]
    Scope(String),
}

impl CatalogueError {
    /// Wire-format discriminant carried in [`ProviderError::kind`] (ADR-019
    /// Amendment 1). Lets consumers branch typed across all four SDKs via
    /// a single string compare.
    pub fn kind(&self) -> &'static str {
        match self {
            CatalogueError::NotSupported => "not_supported",
            CatalogueError::Unavailable(_) => "unavailable",
            CatalogueError::Scope(_) => "scope",
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

/// Aggregate live results across configured providers. Errors land in
/// `result.errors` as typed `ProviderError` per Amendment 1. Sequential
/// for-loop today — a Rust `Client` carries one provider's credentials,
/// so the `n in {0, 1}` reality means `futures::join_all` would not
/// change observed runtime.
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
            Ok(ms) => all.extend(ms),
            Err(err) => {
                errors.insert(
                    provider_name_slug(p.name).to_string(),
                    ProviderError {
                        kind: err.kind().to_string(),
                        message: err.to_string(),
                    },
                );
            }
        }
    }
    if let Some(c) = models.cap_filter {
        all.retain(|m| m.capabilities.contains(&c));
    }
    all.sort_by(|a, b| {
        let pa = provider_name_slug(a.provider.name);
        let pb = provider_name_slug(b.provider.name);
        pa.cmp(pb).then_with(|| a.id.cmp(&b.id))
    });
    LiveResult { models: all, errors }
}

/// Single-provider live HTTP. Paginates per the catalogue config until
/// the parser reports no next cursor, then enriches each record with
/// the ontology-derived capability list. Middleware fires once per call
/// (not per page) for observability at the call granularity.
pub(crate) async fn catalogue_run_list(
    scoped: &ScopedModels,
) -> Result<Vec<ModelInfo>, CatalogueError> {
    let cfg = catalogue_config(scoped.target.name).ok_or(CatalogueError::NotSupported)?;
    let pcfg = provider_config(scoped.target.name);

    let base_event = build_event(scoped.target.name, "");
    let mws: &[MiddlewareFn] = &[];
    fire_pre(mws, &base_event)
        .map_err(|veto| CatalogueError::Unavailable(format!("middleware veto: {veto}")))?;
    let effective = effective_provider(scoped);
    let result = paginate(&effective, pcfg, cfg).await;
    fire_post(mws, &base_event);
    let records = result?;
    Ok(enrich(scoped, records))
}

/// Single-provider live model fetch. URL shapes pinned in plan 025
/// (Anthropic `/v1/models/{id}`, OpenAI `/v1/models/{id}`, Google
/// `/v1beta/models/{id}` — the parser strips `models/` from the
/// response, the URL uses the bare ID).
pub(crate) async fn catalogue_run_get(
    scoped: &ScopedModels,
    id: &str,
) -> Result<ModelInfo, CatalogueError> {
    let cfg = catalogue_config(scoped.target.name).ok_or(CatalogueError::NotSupported)?;
    if cfg.parser_kind == "ParseVertexModels" || cfg.parser_kind == "ParseBedrockModels" {
        return Err(CatalogueError::NotSupported);
    }
    let pcfg = provider_config(scoped.target.name);

    let base_event = build_event(scoped.target.name, id);
    let mws: &[MiddlewareFn] = &[];
    fire_pre(mws, &base_event)
        .map_err(|veto| CatalogueError::Unavailable(format!("middleware veto: {veto}")))?;
    let effective = effective_provider(scoped);
    let endpoint_with_id = format!("{}/{}", cfg.endpoint, id);
    let body = fetch_catalogue_url(&effective, pcfg, &endpoint_with_id).await;
    fire_post(mws, &base_event);
    let body = body?;
    let record = parse_single_record(cfg.parser_kind, &body)?;
    Ok(enrich(scoped, vec![record]).into_iter().next().unwrap())
}

// === Providers-namespace runtime (hand-coded mirror of go/providers.go) ===

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

pub(crate) fn catalogue_providers_supported() -> Vec<Provider> {
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

// === HTTP internals ===

/// Build an effective `Provider` for HTTP from the Client's stored
/// credentials (carries `with_base_url` overrides + the API key), not
/// from the user-supplied `scoped.target`. The target only carries the
/// provider name; the credentials live on `client.provider`.
fn effective_provider(scoped: &ScopedModels) -> Provider {
    Provider {
        name: scoped.target.name,
        api_key: scoped.client.provider.api_key.clone(),
        model: None,
        base_url: scoped.client.provider.base_url.clone(),
    }
}

fn build_event(provider: ProviderName, model: &str) -> Event {
    // Use Default for fields not relevant to this op so we don't need
    // to enumerate every Event field that other ops use (tool/args/result).
    Event {
        op: MiddlewareOp::ModelsList,
        provider: provider_name_slug(provider).to_string(),
        model: model.to_string(),
        ..Default::default()
    }
}

async fn paginate(
    provider: &Provider,
    pcfg: &ProviderConfig,
    cfg: &CatalogueConfig,
) -> Result<Vec<ParsedModelRecord>, CatalogueError> {
    let mut cursor = String::new();
    let mut all: Vec<ParsedModelRecord> = Vec::new();
    loop {
        let endpoint = append_cursor(cfg.endpoint, cfg.pagination, &cursor);
        let body = fetch_catalogue_url(provider, pcfg, &endpoint).await?;
        let page = dispatch_parser(cfg.parser_kind, &body)?;
        all.extend(page.records);
        if page.next_cursor.is_empty() {
            return Ok(all);
        }
        cursor = page.next_cursor;
    }
}

fn append_cursor(endpoint: &str, pagination: &str, cursor: &str) -> String {
    if cursor.is_empty() {
        return endpoint.to_string();
    }
    let sep = if endpoint.contains('?') { '&' } else { '?' };
    match pagination {
        "CursorByLastID" => format!(
            "{endpoint}{sep}after_id={}",
            urlencode(cursor)
        ),
        "CursorOpaqueToken" => format!(
            "{endpoint}{sep}pageToken={}",
            urlencode(cursor)
        ),
        _ => endpoint.to_string(),
    }
}

/// Minimal percent-encoder for the cursor-token use case. Avoids pulling
/// in `urlencoding` for one call site; matches RFC 3986 unreserved
/// characters.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.bytes() {
        match ch {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(ch as char)
            }
            _ => out.push_str(&format!("%{:02X}", ch)),
        }
    }
    out
}

async fn fetch_catalogue_url(
    provider: &Provider,
    pcfg: &ProviderConfig,
    endpoint: &str,
) -> Result<String, CatalogueError> {
    let url = build_catalogue_url(provider, pcfg, endpoint);
    let headers = build_catalogue_headers(provider, pcfg);
    let (status, text) = get_text(&url, &headers)
        .await
        .map_err(|err| CatalogueError::Unavailable(err.to_string()))?;
    if status.is_success() {
        return Ok(text);
    }
    if status.as_u16() == 403 && scope_body_matches(&text) {
        return Err(CatalogueError::Scope(format!("status {status}")));
    }
    Err(CatalogueError::Unavailable(format!("status {status}")))
}

fn scope_body_matches(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("scope") || lower.contains("permission")
}

fn build_catalogue_url(provider: &Provider, pcfg: &ProviderConfig, endpoint: &str) -> String {
    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| pcfg.base_url.to_string());
    let mut full = format!("{base}{endpoint}");
    if matches!(auth_scheme(provider.name), AuthScheme::QueryParamKey) {
        let sep = if full.contains('?') { '&' } else { '?' };
        full = format!(
            "{full}{sep}{}={}",
            pcfg.auth_query_param,
            urlencode(&provider.api_key)
        );
    }
    full
}

fn build_catalogue_headers(provider: &Provider, pcfg: &ProviderConfig) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    match auth_scheme(provider.name) {
        AuthScheme::BearerToken => headers.push((
            pcfg.auth_header.to_string(),
            format!("{} {}", pcfg.auth_prefix, provider.api_key),
        )),
        AuthScheme::HeaderApiKey => {
            headers.push((pcfg.auth_header.to_string(), provider.api_key.clone()))
        }
        AuthScheme::QueryParamKey | AuthScheme::SigV4 => {}
    }
    if !pcfg.required_header.is_empty() {
        headers.push((
            pcfg.required_header.to_string(),
            pcfg.required_header_value.to_string(),
        ));
    }
    headers
}

fn dispatch_parser(kind: &str, body: &str) -> Result<ParsedModelsPage, CatalogueError> {
    let bytes = body.as_bytes();
    let result = match kind {
        "ParseAnthropicModels" => parse_anthropic_models_response(bytes),
        "ParseGoogleModels" => parse_google_models_response(bytes),
        "ParseOpenAICohortModels" => parse_openai_cohort_models_response(bytes),
        _ => return Err(CatalogueError::NotSupported),
    };
    result.map_err(|e| CatalogueError::Unavailable(format!("parse {kind}: {e}")))
}

fn parse_single_record(kind: &str, body: &str) -> Result<ParsedModelRecord, CatalogueError> {
    let wrapped = match kind {
        "ParseAnthropicModels" => format!(r#"{{"data":[{body}]}}"#),
        "ParseGoogleModels" => format!(r#"{{"models":[{body}]}}"#),
        "ParseOpenAICohortModels" => format!(r#"{{"data":[{body}]}}"#),
        _ => return Err(CatalogueError::NotSupported),
    };
    let page = dispatch_parser(kind, &wrapped)?;
    page.records
        .into_iter()
        .next()
        .ok_or_else(|| CatalogueError::Unavailable("empty single-record response".to_string()))
}

fn enrich(scoped: &ScopedModels, records: Vec<ParsedModelRecord>) -> Vec<ModelInfo> {
    records
        .into_iter()
        .map(|rec| {
            let caps = ontology_capabilities(scoped.target.name, &rec.id)
                .map(|s| s.to_vec())
                .unwrap_or_default();
            let raw = if scoped.raw_flag { rec.raw } else { None };
            ModelInfo {
                id: rec.id,
                provider: Provider {
                    name: scoped.target.name,
                    api_key: String::new(),
                    model: None,
                    base_url: None,
                },
                capabilities: caps,
                display_name: rec.display_name,
                description: rec.description,
                context_window: rec.context_window,
                max_output: rec.max_output,
                created: rec.created,
                raw,
            }
        })
        .collect()
}

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

fn provider_name_slug(name: ProviderName) -> &'static str {
    crate::providers::generated::providers::provider_config(name).slug
}
