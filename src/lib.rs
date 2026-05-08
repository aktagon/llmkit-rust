mod agent;
mod batch;
pub mod builders;
mod caching;
mod error;
mod http;
mod image;
mod middleware;
mod options;
mod paths;
pub mod providers;
mod request;
mod response;
mod sigv4;
mod stream;
mod transforms;
mod types;
mod uploads;

pub use agent::Agent;
pub use batch::{prompt_batch, submit_batch, wait_batch, BatchHandle};
pub use error::Error;
pub use image::{
    generate_image, ImageData, ImageOptions, ImageRequest, ImageResponse, MediaRef, Part,
};
pub use middleware::{
    fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase, MiddlewareVeto,
};
pub use options::PromptOptions;
pub use providers::generated::batch::{batch_config, BatchDef, BatchInputMode};
pub use providers::generated::caching::{
    cache_usage_paths, caching_config, CachingDef, CachingMode, ResourceLifecycleDef,
};
pub use providers::generated::image_gen::{image_gen_config, ImageGenDef, ImageModelDef};
pub use providers::generated::options::{
    option_overrides, supported_options, OptionDef, OptionKey, OptionOverrideDef,
    SupportedOptionDef, ALL_OPTIONS,
};
pub use providers::generated::providers::{
    provider_config, ProviderConfig, ProviderName, ALL_PROVIDER_NAMES, PROVIDERS,
};
pub use providers::generated::request::{
    auth_scheme, file_upload_config, structured_output, system_placement, AuthScheme,
    FileUploadDef, StructuredOutputDef, SystemPlacement, ToolCallDef,
};
pub use providers::generated::response::{response_text_path, usage_paths};
pub use providers::generated::stream::{stream_config, StreamDef};
pub use types::{File, InputImage, Message, Provider, Request, Response, Tool, Usage};
pub use uploads::upload_file;

pub async fn prompt(
    provider: &Provider,
    request: &Request,
    options: PromptOptions,
) -> Result<Response, Error> {
    crate::request::validate_provider(provider)?;
    crate::request::validate_request(request)?;
    crate::request::validate_options(provider, &options)?;

    let config = provider_config(provider.name);
    let model = provider
        .model
        .clone()
        .unwrap_or_else(|| config.default_model.to_string());
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
    let url = crate::request::build_url(provider, config);
    let (mut body, headers) = crate::request::build_request(provider, request, &options)?;
    crate::caching::apply_caching(&mut body, provider, &options, config).await?;

    let (status, response_body) =
        if matches!(crate::auth_scheme(provider.name), crate::AuthScheme::SigV4) {
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

    crate::response::parse_response(provider, &response_body)
}

pub async fn prompt_stream<F>(
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
