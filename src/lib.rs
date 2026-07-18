//! llmkit — unified LLM client. One API, many providers, zero deps.
//!
//! The public surface is the typed builder reachable via
//! `llmkit::builders::Client` plus types + error + middleware
//! re-exports. The legacy free-function layer (`prompt`, `prompt_stream`,
//! `generate_image`, `upload_file`, batch trio, `Agent`) was deleted in
//! v1.0.0 (plan 019); the bodies live on as `pub(crate)` helpers
//! consumed by the typed-builder terminals.

mod agent;
mod batch;
pub mod builders;
mod caching;
pub mod catalogue;
mod error;
mod http;
mod image;
mod job;
mod middleware;
pub mod models;
pub mod models_parsers;
mod music;
mod options;
mod paths;
pub mod providers;
mod speech;
mod request;
mod response;
// Public only for the SigV4 wire-conformance driver (CR-002,
// tests/sigv4_wire.rs); hidden from docs, not part of the public API.
#[doc(hidden)]
pub mod sigv4;
mod stream;
mod structs;
pub mod telemetry;
mod transforms;
mod types;
mod uploads;
mod video;
pub mod wire;
mod wire_version;

// === v1.0.0 public surface ===
//
// Trimmed in plan 020 per pre-release review B7: codegen-internal
// configs (BatchDef, CachingDef, OptionDef, *_config helpers, response
// path tables, AuthScheme, SystemPlacement, ProviderSpec, ...) are
// no longer re-exported at the crate root. They were never part of
// the user-facing API; their public exposure would have locked every
// generated struct field into the SemVer 1.0 contract.
//
// Internal call sites continue to import them via the full
// `crate::providers::generated::*` paths.

/// Common imports for typed-builder callers (ADR-064 AJU-010).
/// Glob-import (`use llmkit::prelude::*`) to bring the async-handle
/// extension traits into scope so `handle.wait()` / `handle.poll()` —
/// and the `submit(...).await?.await?` compose (AJU-007) — resolve
/// without naming each `…HandleExt` trait explicitly.
pub mod prelude {
    pub use crate::builders::{BatchHandleExt, TranscriptionHandleExt, VideoHandleExt};
}

pub use error::Error;
pub use image::{ImageData, ImageOptions, ImageRequest, MediaRef, Part};
pub use job::{JobFailure, JobState, JobStatus};
pub use music::{MusicOptions, MusicRequest};
pub use batch::{wait_batch, BatchPoll};
pub use builders::{wait_transcription, TranscriptionPoll};
pub use video::{submit_video, wait_video, VideoPoll, VideoRequest};
pub use structs::{
    AudioData, BatchHandle, File, ImageResponse, LiveResult, Message, ModelInfo, MusicResponse,
    Response, ToolCall, ToolResult, VideoData, VideoHandle, VideoResponse,
};
pub use middleware::{Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase, MiddlewareVeto};
pub use telemetry::{build_otlp_traces, http_export, Telemetry, TelemetryExport};
pub use wire::{load_history, save_history, WireError};
pub use wire_version::WIRE_SCHEMA_VERSION;
pub use models::CatalogueError;
pub use types::Capability;
pub use options::PromptOptions;
pub use request::RESPONSES;
pub use providers::generated::providers::{
    ProviderName, UnknownProviderError, ALL_PROVIDER_NAMES,
};
pub use types::{
    SafetySetting, Tool, Usage,
    HARM_BLOCK_THRESHOLD_HIGH_ONLY, HARM_BLOCK_THRESHOLD_LOW_AND_ABOVE,
    HARM_BLOCK_THRESHOLD_MEDIUM_AND_ABOVE, HARM_BLOCK_THRESHOLD_NONE,
    HARM_CATEGORY_CIVIC_INTEGRITY, HARM_CATEGORY_DANGEROUS_CONTENT,
    HARM_CATEGORY_HARASSMENT, HARM_CATEGORY_HATE_SPEECH,
    HARM_CATEGORY_SEXUALLY_EXPLICIT, IMAGE_SAFETY_FILTER_BLOCK_FEW,
    IMAGE_SAFETY_FILTER_BLOCK_MOST, IMAGE_SAFETY_FILTER_BLOCK_ONLY_HIGH,
    IMAGE_SAFETY_FILTER_BLOCK_SOME,
};

// Internal re-exports — only the names actually reached via
// `crate::*` shortcuts. Other generated symbols (BatchDef, CachingDef,
// ToolCallDef, ...) are imported by their owning module via the full
// `crate::providers::generated::*` path on demand.
pub(crate) use middleware::{fire_post, fire_pre};
pub(crate) use providers::generated::caching::ResourceLifecycleDef;
pub(crate) use providers::generated::options::SupportedOptionDef;
pub(crate) use providers::generated::providers::{provider_config, ProviderSpec};
pub(crate) use providers::generated::request::{auth_scheme, AuthScheme};
pub(crate) use providers::generated::response::{response_text_path, usage_paths};
pub use types::Provider;
pub(crate) use types::Request;

pub(crate) async fn prompt(
    provider: &Provider,
    request: &Request,
    options: PromptOptions,
) -> Result<Response, Error> {
    crate::request::validate_provider(provider)?;
    crate::request::validate_request(request)?;
    crate::request::validate_options(provider, &options)?;

    let config = provider_config(provider.name);
    let model = crate::request::resolve_model(provider, config)?;
    let base_event = Event {
        op: MiddlewareOp::LlmRequest,
        provider: format!("{:?}", provider.name),
        model,
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(&options.middleware, &base_event)?;

    let mws = options.middleware.clone();
    let result = prompt_inner(provider, request, options).await;

    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    match &result {
        Ok(resp) => {
            post_event.usage = Some(crate::middleware::Usage {
                input: resp.usage.input as i64,
                output: resp.usage.output as i64,
                cache_write: resp.usage.cache_write as i64,
                cache_read: resp.usage.cache_read as i64,
                reasoning: resp.usage.reasoning as i64,
                cost: resp.usage.cost,
            })
        }
        Err(err) => post_event.err = Some(err.to_string()),
    }
    fire_post(&mws, &post_event);
    result
}

async fn prompt_inner(
    provider: &Provider,
    request: &Request,
    options: PromptOptions,
) -> Result<Response, Error> {
    let config = provider_config(provider.name);
    // ADR-055: resolve the effective chat protocol (Protocol(...) opt-in) so the
    // URL targets the protocol's endpoint (e.g. /v1/responses) and the response
    // is parsed with the effective wire shape. An unknown/unsupported protocol
    // errors here, before any network call. build_request resolves the same
    // token internally for the body envelope.
    let effective =
        crate::request::resolve_chat_protocol(config, options.protocol.as_deref().unwrap_or(""))?;
    let url = crate::request::build_url(provider, &effective);
    let msgs = crate::transforms::to_internal(&request.messages)?;
    let (mut body, headers) = crate::request::build_request(provider, request, &msgs, &options, &[])?;
    crate::caching::apply_caching(&mut body, provider, &options, config).await?;

    let (status, response_body) =
        if matches!(crate::auth_scheme(provider.name), crate::AuthScheme::SigV4) {
            // ADR-052: caller custom headers ride alongside the signed
            // Bedrock request (added post-signing).
            let caller_headers: Vec<(String, String)> = provider
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let region = std::env::var(config.region_env_var).map_err(|_| Error::Validation {
                field: "provider",
                message: format!("missing env var {}", config.region_env_var),
            })?;
            let secret_key =
                std::env::var(config.secret_key_env_var).map_err(|_| Error::Validation {
                    field: "provider",
                    message: format!("missing env var {}", config.secret_key_env_var),
                })?;
            let session_token = if config.session_token_env_var.is_empty() {
                String::new()
            } else {
                std::env::var(config.session_token_env_var).unwrap_or_default()
            };
            crate::http::post_json_sigv4(
                &url,
                body,
                &provider.api_key,
                &secret_key,
                &session_token,
                &region,
                config.service_name,
                &caller_headers,
            )
            .await?
        } else {
            crate::http::post_json(&url, body, &headers).await?
        };
    if !status.is_success() {
        return Err(crate::response::parse_api_error(
            provider,
            status.as_u16(),
            &response_body,
        ));
    }

    let mut resp =
        crate::response::parse_response_shaped(provider, effective.chat_wire_shape, &response_body)?;
    if options.raw {
        resp.raw = serde_json::from_str(&response_body).ok();
    }
    Ok(resp)
}

pub(crate) async fn prompt_stream<F>(
    provider: &Provider,
    request: &Request,
    options: PromptOptions,
    callback: F,
) -> Result<Response, Error>
where
    F: FnMut(&str),
{
    crate::request::validate_provider(provider)?;
    crate::request::validate_request(request)?;
    crate::request::validate_options(provider, &options)?;
    crate::stream::prompt_stream(provider, request, &options, callback).await
}
