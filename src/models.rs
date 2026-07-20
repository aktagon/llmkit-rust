//!
//!
//!
//!
//!
//!

use crate::builders::Client;
use crate::builders::catalogue::{Models, ScopedModels};
use crate::catalogue::{
    catalogue_config, ontology_capabilities, CatalogueConfig, COMPILED_IN_MODELS,
};
use crate::http::get_text;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::providers::generated::provider_info::{info, ProviderInfo};
use crate::providers::generated::providers::{
    provider_config, ProviderSpec, ProviderName,
};
use crate::models_parsers::{
    parse_anthropic_models_response, parse_google_models_response,
    parse_openai_cohort_models_response, ParsedModelRecord, ParsedModelsPage,
};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::structs::{LiveResult, ModelInfo, ProviderError};
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
#
pub enum CatalogueError {
    #
    NotSupported,
    #
    Unavailable(String),
    #
    Scope(String),
}

impl CatalogueError {
    ///
    ///
    ///
    pub fn kind(&self) -> &'static str {
        match self {
            CatalogueError::NotSupported => "not_supported",
            CatalogueError::Unavailable(_) => "unavailable",
            CatalogueError::Scope(_) => "scope",
        }
    }
}

///
///
///
///
///
pub(crate) fn apply_cap_filter(
    mut models: Vec<ModelInfo>,
    cap_filter: Option<Capability>,
) -> Vec<ModelInfo> {
    if let Some(c) = cap_filter {
        models.retain(|m| m.capabilities.contains(&c));
    }
    models
}

///
///
pub(crate) fn catalogue_filter(cap_filter: Option<Capability>) -> Vec<ModelInfo> {
    apply_cap_filter(
        COMPILED_IN_MODELS.iter().map(compiled_to_model_info).collect(),
        cap_filter,
    )
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
///
///
pub(crate) async fn catalogue_run_live(models: &Models) -> LiveResult {
    use std::collections::HashMap;
    let configured = models.client.providers().list();
    let mut all: Vec<ModelInfo> = Vec::new();
    let mut errors: HashMap<String, ProviderError> = HashMap::new();
    for p in configured {
        let target = Provider {
            name: p.id,
            api_key: String::new(),
            model: None,
            base_url: None,
            headers: std::collections::HashMap::new(),
        };
        let scoped = ScopedModels {
            client: models.client.clone(),
            target,
            cap_filter: models.cap_filter,
            raw_flag: false,
        };
        match scoped.list().await {
            Ok(ms) => all.extend(ms),
            Err(err) => {
                errors.insert(
                    provider_name_slug(p.id).to_string(),
                    ProviderError {
                        kind: err.kind().to_string(),
                        message: err.to_string(),
                    },
                );
            }
        }
    }
    //
    //
    all.sort_by(|a, b| {
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
///
///
pub(crate) async fn catalogue_run_list(
    scoped: &ScopedModels,
) -> Result<Vec<ModelInfo>, CatalogueError> {
    let cfg = catalogue_config(scoped.target.name).ok_or(CatalogueError::NotSupported)?;
    let pcfg = provider_config(scoped.target.name);

    let base_event = build_event(scoped.target.name, "");
    //
    //
    let mws: &[MiddlewareFn] = &scoped.client.default_middleware;
    fire_pre(mws, &base_event)
        .map_err(|veto| CatalogueError::Unavailable(format!("middleware veto: {veto}")))?;
    let start = std::time::Instant::now();
    let effective = effective_provider(scoped);
    let result = paginate(&effective, pcfg, cfg).await;
    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &result {
        post_event.err = Some(err.to_string());
        //
        //
        post_event.err_type = "error".to_string();
    }
    fire_post(mws, &post_event);
    let records = result?;
    Ok(apply_cap_filter(enrich(scoped, records), scoped.cap_filter))
}

///
///
///
///
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
    //
    let mws: &[MiddlewareFn] = &scoped.client.default_middleware;
    fire_pre(mws, &base_event)
        .map_err(|veto| CatalogueError::Unavailable(format!("middleware veto: {veto}")))?;
    let start = std::time::Instant::now();
    let effective = effective_provider(scoped);
    let endpoint_with_id = format!("{}/{}", cfg.endpoint, id);
    let body = fetch_catalogue_url(&effective, pcfg, &endpoint_with_id, "", "").await;
    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &body {
        post_event.err = Some(err.to_string());
        //
        //
        post_event.err_type = "error".to_string();
    }
    fire_post(mws, &post_event);
    let body = body?;
    let record = parse_single_record(cfg.parser_kind, &body)?;
    Ok(enrich(scoped, vec![record]).into_iter().next().unwrap())
}

//

pub(crate) fn catalogue_providers_list(client: &Client) -> Vec<&'static ProviderInfo> {
    if catalogue_config(client.provider.name).is_none() {
        return Vec::new();
    }
    vec![info(client.provider.name)]
}

//

///
///
///
///
fn effective_provider(scoped: &ScopedModels) -> Provider {
    Provider {
        name: scoped.target.name,
        api_key: scoped.client.provider.api_key.clone(),
        model: None,
        base_url: scoped.client.provider.base_url.clone(),
        headers: scoped.client.provider.headers.clone(),
    }
}

fn build_event(provider: ProviderName, model: &str) -> Event {
    //
    //
    Event {
        op: MiddlewareOp::ModelsList,
        provider: provider_name_slug(provider).to_string(),
        model: model.to_string(),
        ..Default::default()
    }
}

async fn paginate(
    provider: &Provider,
    pcfg: &ProviderSpec,
    cfg: &CatalogueConfig,
) -> Result<Vec<ParsedModelRecord>, CatalogueError> {
    let mut cursor = String::new();
    let mut all: Vec<ParsedModelRecord> = Vec::new();
    loop {
        let body = fetch_catalogue_url(provider, pcfg, cfg.endpoint, &cursor, cfg.cursor_param).await?;
        let page = dispatch_parser(cfg.parser_kind, &body)?;
        all.extend(page.records);
        if page.next_cursor.is_empty() {
            return Ok(all);
        }
        cursor = page.next_cursor;
    }
}

//
//
//
//
//
//
fn append_cursor(raw_url: &str, cursor_param: &str, cursor: &str) -> String {
    if cursor.is_empty() || cursor_param.is_empty() {
        return raw_url.to_string();
    }
    let sep = if raw_url.contains('?') { '&' } else { '?' };
    format!("{raw_url}{sep}{cursor_param}={}", urlencode(cursor))
}

///
///
///
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
    pcfg: &ProviderSpec,
    endpoint: &str,
    cursor: &str,
    cursor_param: &str,
) -> Result<String, CatalogueError> {
    //
    //
    //
    let url = append_cursor(
        &build_catalogue_url(provider, pcfg, endpoint),
        cursor_param,
        cursor,
    );
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

fn build_catalogue_url(provider: &Provider, pcfg: &ProviderSpec, endpoint: &str) -> String {
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

fn build_catalogue_headers(provider: &Provider, pcfg: &ProviderSpec) -> Vec<(String, String)> {
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
    //
    //
    for (k, v) in &provider.headers {
        if !headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(k)) {
            headers.push((k.clone(), v.clone()));
        }
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
                    headers: std::collections::HashMap::new(),
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
            headers: std::collections::HashMap::new(),
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

//
//
//
//
//
//
//
//
//
//
//
//
#
mod catalogue_wire {
    use super::*;
    use std::str::FromStr;

    fn repo_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root")
            .to_path_buf()
    }

    #
    fn catalogue_wire_matches_goldens() {
        let root = repo_root();
        let catalogue_dir = root.join("codegen/testdata/wire/catalogue/v1");
        let inputs: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(catalogue_dir.join("inputs.json")).expect("read inputs"),
        )
        .expect("parse inputs");

        let api_key = inputs["apiKey"].as_str().expect("apiKey");
        let cases = inputs["cases"].as_object().expect("cases");

        for (case, spec) in cases {
            let name = ProviderName::from_str(spec["provider"].as_str().expect("provider"))
                .expect("known provider");
            let cursor = spec["cursor"].as_str().expect("cursor");

            let provider = Provider {
                name,
                api_key: api_key.to_string(),
                model: None,
                base_url: None,
                headers: std::collections::HashMap::new(),
            };
            let pcfg = provider_config(name);
            let cfg = catalogue_config(name).expect("catalogue config");

            let url = append_cursor(
                &build_catalogue_url(&provider, pcfg, cfg.endpoint),
                cfg.cursor_param,
                cursor,
            );
            let headers = build_catalogue_headers(&provider, pcfg);

            let mut header_map = serde_json::Map::new();
            for (k, v) in headers {
                header_map.insert(k, serde_json::Value::String(v));
            }
            let artifact = serde_json::json!({
                "method": "GET",
                "url": url,
                "headers": serde_json::Value::Object(header_map),
            });

            let out = root.join(format!("target/wire/catalogue/{case}/rust.json"));
            std::fs::create_dir_all(out.parent().unwrap()).expect("mkdir artifact dir");
            std::fs::write(&out, serde_json::to_string_pretty(&artifact).unwrap())
                .expect("write artifact");

            let golden: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(catalogue_dir.join(format!("{case}.json")))
                    .expect("read golden"),
            )
            .expect("parse golden");
            assert_eq!(artifact, golden, "Rust catalogue {case} differs from golden");
        }
    }
}
