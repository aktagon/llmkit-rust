use serde_json::{json, Value};

use crate::error::Error;
use crate::http::post_json;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareOp};
use crate::options::PromptOptions;
use crate::paths::extract_string_path;
use crate::providers::generated::caching::{caching_config, CachingMode};
use crate::providers::generated::providers::ProviderSpec;
use crate::providers::generated::request::SystemPlacement;
use crate::request::system_placement_for;
use crate::types::Provider;

pub async fn apply_caching(
    body: &mut Value,
    provider: &Provider,
    options: &PromptOptions,
    config: &ProviderSpec,
) -> Result<(), Error> {
    if !options.caching {
        return Ok(());
    }

    let caching = caching_config(provider.name).ok_or_else(|| Error::Validation {
        field: "caching",
        message: format!("not supported by {:?}", provider.name),
    })?;

    match caching.mode {
        CachingMode::AutomaticCaching => Ok(()),
        CachingMode::ExplicitCaching => apply_explicit_caching(body, caching.control_type, config),
        CachingMode::ResourceCaching => apply_resource_caching(body, provider, options, config).await,
    }
}

fn apply_explicit_caching(body: &mut Value, control_type: &str, config: &ProviderSpec) -> Result<(), Error> {
    let Some(root) = body.as_object_mut() else {
        return Ok(());
    };
    match system_placement_for(config.name) {
        SystemPlacement::TopLevelField => {
            let Some(system) = root.get("system").and_then(Value::as_str).map(str::to_string) else {
                return Ok(());
            };
            root.insert(
                "system".into(),
                json!([{
                    "type": "text",
                    "text": system,
                    "cache_control": {"type": control_type}
                }]),
            );
        }
        SystemPlacement::MessageInArray => {
            let Some(messages) = root.get_mut("messages").and_then(Value::as_array_mut) else {
                return Ok(());
            };
            for message in messages.iter_mut().rev() {
                let Some(object) = message.as_object_mut() else {
                    continue;
                };
                if object.get("role").and_then(Value::as_str) == Some("system") {
                    if let Some(content) = object.get("content").and_then(Value::as_str).map(str::to_string) {
                        object.insert(
                            "content".into(),
                            json!([{
                                "type": "text",
                                "text": content,
                                "cache_control": {"type": control_type}
                            }]),
                        );
                    }
                    break;
                }
            }
        }
        SystemPlacement::SiblingObject => {}
    }
    Ok(())
}

async fn apply_resource_caching(
    body: &mut Value,
    provider: &Provider,
    options: &PromptOptions,
    config: &ProviderSpec,
) -> Result<(), Error> {
    let caching = caching_config(provider.name).and_then(|config| config.lifecycle).ok_or_else(|| {
        Error::Unsupported("resource caching requires lifecycle config".into())
    })?;

    let Some(root) = body.as_object_mut() else {
        return Ok(());
    };
    let Some(system_instruction) = root.get("system_instruction").cloned() else {
        return Ok(());
    };

    let model = crate::request::resolve_model(provider, config)?;
    let ttl = options
        .cache_ttl
        .map(|seconds| seconds.to_string())
        .filter(|seconds| !seconds.is_empty())
        .unwrap_or_else(|| {
            caching_config(provider.name)
                .map(|config| config.default_ttl.to_string())
                .unwrap_or_default()
        });

    let create_url = format!(
        "{}{}?{}={}",
        provider
            .base_url
            .clone()
            .unwrap_or_else(|| config.base_url.to_string()),
        caching.create_endpoint,
        config.auth_query_param,
        provider.api_key
    );

    let create_body = json!({
        "model": format!("models/{model}"),
        "ttl": format!("{ttl}s"),
        "contents": [{"role": "user", "parts": [{"text": "cache"}]}],
        "systemInstruction": system_instruction,
    });

    let base_event = Event {
        op: MiddlewareOp::CacheCreate,
        provider: format!("{:?}", provider.name),
        model: model.clone(),
        ..Event::default()
    };
    let start = std::time::Instant::now();
    fire_pre(&options.middleware, &base_event)?;

    // ADR-052: Google resource caching authenticates via a query param, so
    // there is no auth header to collide — forward the caller custom headers.
    let caller_headers: Vec<(String, String)> = provider
        .headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let outcome: Result<String, Error> = (async {
        let (status, response_body) =
            post_json(&create_url, create_body, &caller_headers).await?;
        if !status.is_success() {
            return Err(crate::response::parse_api_error(
                provider,
                status.as_u16(),
                &response_body,
            ));
        }
        let parsed: Value = serde_json::from_str(&response_body)?;
        let resource_id = extract_string_path(&parsed, caching.response_id_path);
        if resource_id.is_empty() {
            return Err(Error::Unsupported("cache create: empty resource ID".into()));
        }
        Ok(resource_id)
    })
    .await;

    let mut post_event = base_event.clone();
    post_event.duration = Some(start.elapsed());
    if let Err(err) = &outcome {
        post_event.err = Some(err.to_string());
    }
    fire_post(&options.middleware, &post_event);

    let resource_id = outcome?;
    root.insert(
        caching.reference_field.to_string(),
        Value::String(resource_id),
    );
    root.remove("system_instruction");
    Ok(())
}
