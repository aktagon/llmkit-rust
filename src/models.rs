//! Hand-coded catalogue runtime (ADR-019). The generated builder types
//! in `builders/catalogue.rs` delegate their terminal methods here.
//!
//! Folds in the providers-namespace runtime (`catalogue_providers_*`)
//! because `crate::providers` is the generated subpackage path and Rust
//! forbids shadowing it with a sibling module.

use crate::builders::Client;
use crate::builders::{Models, ScopedModels};
use crate::catalogue::{
    catalogue_config, ontology_capabilities, CatalogueConfig, COMPILED_IN_MODELS,
};
use crate::http::get_text;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::model_lookup::lookup_model;
use crate::models_parsers::{
    parse_anthropic_models_response, parse_google_models_response,
    parse_openai_cohort_models_response, ParsedModelRecord, ParsedModelsPage,
};
use crate::providers::generated::image_gen::{image_gen_config, ImageModelDef};
use crate::providers::generated::music_gen::{music_gen_config, MusicModelDef};
use crate::providers::generated::provider_info::{info, ProviderInfo};
use crate::providers::generated::providers::{provider_config, ProviderName, ProviderSpec};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::providers::generated::speech_gen::{speech_gen_config, SpeechGenDef, SpeechModelDef};
use crate::providers::generated::video_gen::{video_gen_config, VideoModelDef};
use crate::structs::{
    ImageHints, LiveResult, ModelInfo, MusicHints, ProviderError, SpeechHints, VideoHints,
};
use crate::types::{Capability, Provider};

/// Catalogue error sentinels (ADR-019). Live provider calls map to one
/// of these variants:
///
/// * [`CatalogueError::NotSupported`] — provider lacks
///   `hasModelsEndpoint` (no `/v1/models` route; nothing to fetch).
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

/// Records whose capabilities contain the filter; identity when `None`.
/// The single capability predicate (HANDOFF-036 A4): shared by the
/// scoped live list (`catalogue_run_list`) and — through it — the live
/// aggregate.
/// `get` stays an unfiltered point lookup by id.
pub(crate) fn apply_cap_filter(
    mut models: Vec<ModelInfo>,
    cap_filter: Option<Capability>,
) -> Vec<ModelInfo> {
    if let Some(c) = cap_filter {
        models.retain(|m| m.capabilities.contains(&c));
    }
    models
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
        let target = Provider {
            name: p.id,
            api_key: String::new(),
            model: None,
            base_url: None,
            headers: std::collections::HashMap::new(),
            timeout: crate::builders::DEFAULT_TIMEOUT,
        };
        match models.provider(target).list().await {
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
    // cap_filter is already applied per-provider inside scoped.list()
    // (HANDOFF-036 A4) — no aggregate re-filter needed.
    all.sort_by(|a, b| {
        let pa = provider_name_slug(a.provider.name);
        let pb = provider_name_slug(b.provider.name);
        pa.cmp(pb).then_with(|| a.id.cmp(&b.id))
    });
    LiveResult {
        models: all,
        errors,
    }
}

///
/// capability list and filtered by the chain's `cap_filter`
/// (`with_capability` composes with `provider(p).list()` — HANDOFF-036
/// A4; `get` stays an unfiltered point lookup by id). A list younger than
/// one hour comes from the client's cache unless `fresh()` is set
/// (ADR-102 MPT-005); otherwise the live call runs and a successful
/// result refills the cache.
pub(crate) async fn catalogue_run_list(
    scoped: &ScopedModels,
) -> Result<Vec<ModelInfo>, CatalogueError> {
    let records = match cached_records(scoped) {
        Some(records) => records,
        None => {
            let records = fetch_list(scoped).await?;
            scoped
                .client
                .model_cache
                .put(&cache_key(scoped), records.clone());
            records
        }
    };
    Ok(apply_cap_filter(enrich(scoped, records), scoped.cap_filter))
}

/// The provider's cached records, unless the chain asked for a fresh call.
fn cached_records(scoped: &ScopedModels) -> Option<Vec<ParsedModelRecord>> {
    if scoped.fresh_flag {
        return None;
    }
    scoped.client.model_cache.get(&cache_key(scoped))
}

/// Names the cached list by the provider and the base URL and API key the
/// live call is sent with (owner, 2026-11-01). Clones of a `Client` share
/// the cache, so a clone pointed at another endpoint or key never reads a
/// list fetched for the first one. The key stays in memory only.
fn cache_key(scoped: &ScopedModels) -> String {
    let effective = effective_provider(scoped);
    let base = effective
        .base_url
        .unwrap_or_else(|| provider_config(scoped.target.name).base_url.to_string());
    format!(
        "{}\0{}\0{}",
        provider_name_slug(scoped.target.name),
        base,
        effective.api_key
    )
}

/// Single-provider live HTTP. Paginates per the catalogue config until
/// the parser reports no next cursor. Middleware fires once per call (not
/// per page) for observability at the call granularity. A list served
/// from the cache makes no call and fires no event.
async fn fetch_list(scoped: &ScopedModels) -> Result<Vec<ParsedModelRecord>, CatalogueError> {
    let cfg = catalogue_config(scoped.target.name).ok_or(CatalogueError::NotSupported)?;
    let pcfg = provider_config(scoped.target.name);

    let base_event = build_event(scoped.target.name, "");
    // Client-scoped hooks (telemetry, ADR-054) observe catalogue calls too
    // (HANDOFF-036 A3); the Swift seam is the reference.
    let mws: &[MiddlewareFn] = &scoped.client.default_middleware;
    fire_pre(mws, &base_event)
        .map_err(|veto| CatalogueError::Unavailable(format!("middleware veto: {veto}")))?;
    let start = crate::clock::Instant::now();
    let effective = effective_provider(scoped);
    let result = paginate(&effective, pcfg, cfg).await;
    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &result {
        post_event.err = Some(err.to_string());
        // CatalogueError carries no crate Error variant (the typed cause is
        // already erased into its message), so the kind is the catch-all.
        post_event.err_type = "error".to_string();
    }
    fire_post(mws, &post_event);
    result
}

/// One model record by id. A fresh cached list that holds the id answers
/// without a call (ADR-102 MPT-005); otherwise the live point lookup
/// runs. URL shapes pinned in plan 025
/// (Anthropic `/v1/models/{id}`, OpenAI `/v1/models/{id}`, Google
/// `/v1beta/models/{id}` — the parser strips `models/` from the
/// response, the URL uses the bare ID).
pub(crate) async fn catalogue_run_get(
    scoped: &ScopedModels,
    id: &str,
) -> Result<ModelInfo, CatalogueError> {
    if let Some(hit) = cached_records(scoped).and_then(|rs| rs.into_iter().find(|r| r.id == id)) {
        return Ok(enrich(scoped, vec![hit]).into_iter().next().unwrap());
    }
    let cfg = catalogue_config(scoped.target.name).ok_or(CatalogueError::NotSupported)?;
    if cfg.parser_kind == "ParseVertexModels" || cfg.parser_kind == "ParseBedrockModels" {
        return Err(CatalogueError::NotSupported);
    }
    let pcfg = provider_config(scoped.target.name);

    let base_event = build_event(scoped.target.name, id);
    // Client-scoped hooks observe catalogue calls (HANDOFF-036 A3).
    let mws: &[MiddlewareFn] = &scoped.client.default_middleware;
    fire_pre(mws, &base_event)
        .map_err(|veto| CatalogueError::Unavailable(format!("middleware veto: {veto}")))?;
    let start = crate::clock::Instant::now();
    let effective = effective_provider(scoped);
    let endpoint_with_id = format!("{}/{}", cfg.endpoint, id);
    let url = build_catalogue_url(&effective, pcfg, &endpoint_with_id);
    let body = fetch_catalogue_url(&effective, pcfg, &url).await;
    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &body {
        post_event.err = Some(err.to_string());
        // CatalogueError carries no crate Error variant (the typed cause is
        // already erased into its message), so the kind is the catch-all.
        post_event.err_type = "error".to_string();
    }
    fire_post(mws, &post_event);
    let body = body?;
    let record = parse_single_record(cfg.parser_kind, &body)?;
    Ok(enrich(scoped, vec![record]).into_iter().next().unwrap())
}

// === Providers-namespace runtime (hand-coded mirror of go/providers.go) ===

pub(crate) fn catalogue_providers_list(client: &Client) -> Vec<&'static ProviderInfo> {
    if catalogue_config(client.provider.name).is_none() {
        return Vec::new();
    }
    vec![info(client.provider.name)]
}

// === HTTP internals ===

/// Build an effective `Provider` for HTTP from the Client's stored
/// credentials (carries `base_url` overrides + the API key), not
/// from the user-supplied `scoped.target`. The target only carries the
/// provider name; the credentials live on `client.provider`.
fn effective_provider(scoped: &ScopedModels) -> Provider {
    Provider {
        name: scoped.target.name,
        api_key: scoped.client.provider.api_key.clone(),
        model: None,
        base_url: scoped.client.provider.base_url.clone(),
        headers: scoped.client.provider.headers.clone(),
        timeout: scoped.client.provider.timeout,
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
    pcfg: &ProviderSpec,
    cfg: &CatalogueConfig,
) -> Result<Vec<ParsedModelRecord>, CatalogueError> {
    let mut cursor = String::new();
    let mut all: Vec<ParsedModelRecord> = Vec::new();
    loop {
        let url = list_page_url(provider, pcfg, cfg, &cursor);
        let body = fetch_catalogue_url(provider, pcfg, &url).await?;
        let page = dispatch_parser(cfg.parser_kind, &body)?;
        all.extend(page.records);
        if page.next_cursor.is_empty() {
            return Ok(all);
        }
        cursor = page.next_cursor;
    }
}

// The URL of one list page: the endpoint, the provider's fixed list query
// (BUG-107: OpenRouter lists text-output models only without
// output_modalities=all), then the cursor. Get-by-id never sends the list query.
fn list_page_url(
    provider: &Provider,
    pcfg: &ProviderSpec,
    cfg: &CatalogueConfig,
    cursor: &str,
) -> String {
    let mut url = build_catalogue_url(provider, pcfg, cfg.endpoint);
    if !cfg.list_query.is_empty() {
        let sep = if url.contains('?') { '&' } else { '?' };
        url = format!("{url}{sep}{}", cfg.list_query);
    }
    append_cursor(&url, cfg.cursor_param, cursor)
}

// Splices the pagination cursor into the URL using the cursor query-param
// name carried by the generated CatalogueConfig (ADR-067 Fix A). Applied to
// the FULL URL after any QueryParamKey `?key=` is spliced in, so a
// query-param-auth provider assembles `?key=...&cursor=...` in the same order
// as Go/Python (CR-003 cross-SDK catalogue-URL byte-parity). An empty cursor
// or an empty cursor_param (PaginationNone) leaves the URL unchanged.
fn append_cursor(raw_url: &str, cursor_param: &str, cursor: &str) -> String {
    if cursor.is_empty() || cursor_param.is_empty() {
        return raw_url.to_string();
    }
    let sep = if raw_url.contains('?') { '&' } else { '?' };
    format!("{raw_url}{sep}{cursor_param}={}", urlencode(cursor))
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
    pcfg: &ProviderSpec,
    url: &str,
) -> Result<String, CatalogueError> {
    let headers = build_catalogue_headers(provider, pcfg);
    let (status, text) = get_text(url, provider.timeout, &headers)
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
    // ADR-052: custom headers reach the catalogue path too; skip any that
    // collide (case-insensitively) with the auth/required header above.
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
            let mut info = catalogue_model_info(scoped.target.name, rec.id);
            info.display_name = rec.display_name;
            info.description = rec.description;
            info.context_window = rec.context_window;
            info.max_output = rec.max_output;
            info.created = rec.created;
            info.raw = if scoped.raw_flag { rec.raw } else { None };
            info
        })
        .collect()
}

/// Every model this SDK version states facts for under `name`, with the
/// capabilities and hints the compiled catalogue holds (ADR-102 MPT-010).
/// It needs no client, no API key and no network. The list is not proof
/// that the provider serves a model; use `models().list()` for that. The
/// live fields (`display_name`, `context_window`, `raw` and the rest) are
/// empty, and the provider carries no API key. Sorted by id.
pub fn catalogue(name: ProviderName) -> Vec<ModelInfo> {
    let mut ids: Vec<&'static str> = COMPILED_IN_MODELS
        .iter()
        .filter(|m| m.provider == name)
        .map(|m| m.id)
        .collect();
    if let Some(cfg) = image_gen_config(name) {
        ids.extend(cfg.models.iter().map(|m| m.model_id));
    }
    if let Some(cfg) = video_gen_config(name) {
        ids.extend(cfg.models.iter().map(|m| m.model_id));
    }
    if let Some(cfg) = speech_gen_config(name) {
        ids.extend(cfg.models.iter().map(|m| m.model_id));
    }
    if let Some(cfg) = music_gen_config(name) {
        ids.extend(cfg.models.iter().map(|m| m.model_id));
    }
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .map(|id| catalogue_model_info(name, id.to_string()))
        .collect()
}

/// The one ModelInfo builder for a live list (`enrich`) and the keyless
/// `catalogue`: the id, the provider without a key, and what the compiled
/// catalogue states about the model. The live fields are left empty.
fn catalogue_model_info(provider: ProviderName, id: String) -> ModelInfo {
    ModelInfo {
        capabilities: ontology_capabilities(provider, &id)
            .map(|s| s.to_vec())
            .unwrap_or_default(),
        image: image_hints(provider, &id),
        video: video_hints(provider, &id),
        speech: speech_hints(provider, &id),
        music: music_hints(provider, &id),
        id,
        provider: Provider {
            name: provider,
            api_key: String::new(),
            model: None,
            base_url: None,
            headers: std::collections::HashMap::new(),
            timeout: crate::builders::DEFAULT_TIMEOUT,
        },
        display_name: String::new(),
        description: String::new(),
        context_window: 0,
        max_output: 0,
        created: 0,
        raw: None,
    }
}

/// What the compiled catalogue states about `model_id` for image
/// generation (ADR-102 MPT-008); `None` when it does not list the model.
fn image_hints(provider: ProviderName, model_id: &str) -> Option<ImageHints> {
    let cfg = image_gen_config(provider)?;
    let (m, _) = lookup_model(cfg.models, model_id, |m| m.model_id);
    m.map(image_hints_of)
}

fn image_hints_of(m: &ImageModelDef) -> ImageHints {
    ImageHints {
        aspect_ratios: strings(m.aspect_ratios),
        sizes: strings(m.image_sizes),
        image_input: m.supports_image_input,
        max_input_images: stated_limit(m.max_input_images),
    }
}

/// What the compiled catalogue states about `model_id` for video
/// generation (ADR-102 MPT-008); `None` when it does not list the model.
fn video_hints(provider: ProviderName, model_id: &str) -> Option<VideoHints> {
    let cfg = video_gen_config(provider)?;
    let (m, _) = lookup_model(cfg.models, model_id, |m| m.model_id);
    m.map(video_hints_of)
}

fn video_hints_of(m: &VideoModelDef) -> VideoHints {
    VideoHints {
        resolutions: strings(m.resolutions),
        aspect_ratios: strings(m.aspect_ratios),
        max_duration_seconds: m.max_duration_seconds,
        image_to_video: m.supports_image_to_video,
        last_frame: m.supports_last_frame,
        reference_images: m.supports_reference_images,
        max_input_images: stated_limit(m.max_input_images),
    }
}

/// What the compiled catalogue states about `model_id` for speech
/// generation (ADR-102 MPT-008); `None` when it does not list the model.
fn speech_hints(provider: ProviderName, model_id: &str) -> Option<SpeechHints> {
    let cfg = speech_gen_config(provider)?;
    let (m, _) = lookup_model(cfg.models, model_id, |m| m.model_id);
    m.map(|m| speech_hints_of(cfg, m))
}

/// The provider's request body limits every model: a body with no style
/// field makes style no, and a body with no multi-speaker form makes one
/// speaker. These are the config checks `validate_speech_model` runs before
/// send (ADR-102 MPT-010).
fn speech_hints_of(cfg: &SpeechGenDef, m: &SpeechModelDef) -> SpeechHints {
    SpeechHints {
        style: if cfg.carries_speech_style {
            m.supports_speech_style
        } else {
            Some(false)
        },
        max_speakers: if cfg.multi_speaker {
            m.max_speakers
        } else {
            Some(1)
        },
    }
}

/// What the compiled catalogue states about `model_id` for music
/// generation (ADR-102 MPT-010); `None` when it does not list the model.
fn music_hints(provider: ProviderName, model_id: &str) -> Option<MusicHints> {
    let cfg = music_gen_config(provider)?;
    let (m, _) = lookup_model(cfg.models, model_id, |m| m.model_id);
    m.map(music_hints_of)
}

fn music_hints_of(m: &MusicModelDef) -> MusicHints {
    MusicHints {
        lyrics: Some(m.supports_lyrics),
        max_duration_seconds: m.max_duration_seconds,
        output_formats: strings(m.available_output_formats),
        image_input: Some(m.supports_image_input),
    }
}

/// The catalogue table holds 0 for a model whose source states no input-image
/// limit (maxInputImages omitted); the hint is then None (ADR-104 PUB-004).
fn stated_limit(n: i64) -> Option<i64> {
    (n > 0).then_some(n)
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn provider_name_slug(name: ProviderName) -> &'static str {
    crate::providers::generated::providers::provider_config(name).slug
}

// Cross-SDK catalogue request-URL conformance (ADR-067 Fix B / CAT-006) — the
// Rust driver. The seam fns (list_page_url / build_catalogue_headers) are private to this module, so the driver lives
// here as a unit-test module rather than an integration test in tests/.
//
// The REQUEST-side sibling of response_wire.rs (which locks the /models PARSE
// seam): for a fixed (provider, cursor), every SDK's catalogue-list path must
// assemble a byte-identical {method, url, headers}. The cursor_param comes from
// the generated catalogue_config, NOT from inputs.json — so this exercises the
// generated config. Drops target/wire/catalogue/<case>/rust.json for
//
// (make check excludes Rust).
// Tests may compare against serde_json; clippy.toml refuses it in library code.











































































































