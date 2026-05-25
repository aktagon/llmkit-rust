use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::options::PromptOptions;
use crate::providers::generated::options::{
    model_option_overrides, option_overrides, supported_options, OptionKey,
};
use crate::providers::generated::providers::{provider_config, ProviderConfig};
use crate::providers::generated::request::{
    auth_scheme, structured_output, system_placement, AuthScheme, SystemPlacement,
};
use crate::types::{Provider, Request};

pub fn validate_provider(provider: &Provider) -> Result<(), Error> {
    if provider.api_key.is_empty() {
        return Err(Error::Validation {
            field: "api_key",
            message: "required".into(),
        });
    }
    Ok(())
}

pub fn validate_request(request: &Request) -> Result<(), Error> {
    if request.user.is_none() && request.messages.is_empty() {
        return Err(Error::Validation {
            field: "user",
            message: "required".into(),
        });
    }
    Ok(())
}

pub fn validate_options(provider: &Provider, options: &PromptOptions) -> Result<(), Error> {
    let supported = supported_options(provider.name);

    validate_option_support(
        options.top_k.is_some(),
        provider,
        supported,
        OptionKey::TopK,
        "top_k",
    )?;
    validate_option_support(
        options.seed.is_some(),
        provider,
        supported,
        OptionKey::Seed,
        "seed",
    )?;
    validate_option_support(
        options.frequency_penalty.is_some(),
        provider,
        supported,
        OptionKey::FrequencyPenalty,
        "frequency_penalty",
    )?;
    validate_option_support(
        options.presence_penalty.is_some(),
        provider,
        supported,
        OptionKey::PresencePenalty,
        "presence_penalty",
    )?;
    validate_option_support(
        options.thinking_budget.is_some(),
        provider,
        supported,
        OptionKey::ThinkingBudget,
        "thinking_budget",
    )?;
    validate_option_support(
        options.reasoning_effort.is_some(),
        provider,
        supported,
        OptionKey::ReasoningEffort,
        "reasoning_effort",
    )?;

    if let Some(value) = options.reasoning_effort.as_deref() {
        if let Some(override_def) = option_overrides(provider.name).iter().find(|entry| {
            entry.key == OptionKey::ReasoningEffort && !entry.allowed_values.is_empty()
        }) {
            if !override_def
                .allowed_values
                .iter()
                .any(|allowed| *allowed == value)
            {
                return Err(Error::Validation {
                    field: "reasoning_effort",
                    message: format!(
                        "invalid value {:?}, must be one of: {}",
                        value,
                        override_def.allowed_values.join(",")
                    ),
                });
            }
        }
    }

    Ok(())
}

fn validate_option_support(
    enabled: bool,
    provider: &Provider,
    supported: &[crate::SupportedOptionDef],
    key: OptionKey,
    field: &'static str,
) -> Result<(), Error> {
    if enabled && !supported.iter().any(|option| option.key == key) {
        return Err(Error::Validation {
            field,
            message: format!("not supported by {:?}", provider.name),
        });
    }
    Ok(())
}

pub fn build_url(provider: &Provider, config: &ProviderConfig) -> String {
    let mut base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| config.base_url.to_string());
    if !config.region_env_var.is_empty() {
        if let Ok(region) = std::env::var(config.region_env_var) {
            base = base.replace("{region}", &region);
        }
    }

    let model = provider
        .model
        .clone()
        .unwrap_or_else(|| config.default_model.to_string());
    let mut endpoint = config.endpoint.replace("{model}", &model);
    endpoint = endpoint.replace("{apiKey}", &provider.api_key);

    if matches!(auth_scheme(provider.name), AuthScheme::QueryParamKey) {
        let separator = if endpoint.contains('?') { "&" } else { "?" };
        endpoint.push_str(separator);
        endpoint.push_str(config.auth_query_param);
        endpoint.push('=');
        endpoint.push_str(&provider.api_key);
    }

    format!("{base}{endpoint}")
}

pub fn system_placement_for(provider: crate::ProviderName) -> SystemPlacement {
    system_placement(provider)
}

pub fn build_auth_headers(provider: &Provider, config: &ProviderConfig) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    match auth_scheme(provider.name) {
        AuthScheme::BearerToken => {
            headers.push((
                config.auth_header.to_string(),
                format!("{} {}", config.auth_prefix, provider.api_key),
            ));
        }
        AuthScheme::HeaderApiKey => {
            headers.push((config.auth_header.to_string(), provider.api_key.clone()));
        }
        AuthScheme::QueryParamKey | AuthScheme::SigV4 => {}
    }
    if !config.required_header.is_empty() {
        headers.push((
            config.required_header.to_string(),
            config.required_header_value.to_string(),
        ));
    }
    headers
}

pub fn build_request(
    provider: &Provider,
    request: &Request,
    options: &PromptOptions,
) -> Result<(Value, Vec<(String, String)>), Error> {
    let config = provider_config(provider.name);

    let model = provider
        .model
        .clone()
        .unwrap_or_else(|| config.default_model.to_string());

    let mut body = Map::new();
    let mut headers = build_auth_headers(provider, config);

    if config.model_in_body {
        body.insert("model".into(), Value::String(model.clone()));
    }

    let max_tokens = options.max_tokens.unwrap_or(config.default_max_tokens);
    if let Some(json_key) = resolve_option_key(provider.name, &model, OptionKey::MaxTokens) {
        body.insert(json_key.to_string(), json!(max_tokens));
    }

    match system_placement(provider.name) {
        SystemPlacement::TopLevelField => {
            if let Some(system) = &request.system {
                if is_bedrock(config) {
                    body.insert("system".into(), json!([{ "text": system }]));
                } else {
                    body.insert("system".into(), Value::String(system.clone()));
                }
            }
        }
        SystemPlacement::MessageInArray => {}
        SystemPlacement::SiblingObject => {
            if let Some(system) = &request.system {
                body.insert(
                    "system_instruction".into(),
                    json!({"parts": [{"text": system}]}),
                );
            }
        }
    }

    apply_message_shape(&mut body, request, config);

    if !config.wraps_options_in.is_empty() {
        let mut wrapped = Map::new();
        add_options(&mut wrapped, provider, &model, options);
        if let Some(json_key) = resolve_option_key(provider.name, &model, OptionKey::MaxTokens) {
            insert_nested_field(&mut wrapped, json_key, json!(max_tokens));
            body.remove(json_key);
        }
        if !wrapped.is_empty() {
            body.insert(config.wraps_options_in.into(), Value::Object(wrapped));
        }
    } else {
        add_options(&mut body, provider, &model, options);
    }

    if !config.safety_settings_wire_path.is_empty() && !options.safety_settings.is_empty() {
        let ss: Vec<Value> = options
            .safety_settings
            .iter()
            .map(|s| json!({"category": s.category, "threshold": s.threshold}))
            .collect();
        body.insert(
            config.safety_settings_wire_path.to_string(),
            Value::Array(ss),
        );
    }

    if let Some(schema) = &request.schema {
        add_structured_output(&mut body, &mut headers, schema, provider.name);
    }

    Ok((Value::Object(body), headers))
}

fn apply_message_shape(body: &mut Map<String, Value>, request: &Request, config: &ProviderConfig) {
    match system_placement(config.name) {
        SystemPlacement::SiblingObject => {
            let mut contents = Vec::new();
            if !request.messages.is_empty() {
                for message in &request.messages {
                    contents.push(json!({
                        "role": map_role(&message.role, config),
                        "parts": [{"text": message.content}],
                    }));
                }
            } else if let Some(user) = &request.user {
                let parts = build_google_parts(request);
                contents.push(json!({
                    "role": map_role("user", config),
                    "parts": parts.unwrap_or_else(|| vec![json!({"text": user})]),
                }));
            }
            body.insert("contents".into(), Value::Array(contents));
        }
        SystemPlacement::TopLevelField | SystemPlacement::MessageInArray => {
            let is_bedrock = is_bedrock(config);
            let mut messages = Vec::new();
            if matches!(
                system_placement(config.name),
                SystemPlacement::MessageInArray
            ) {
                if let Some(system) = &request.system {
                    messages.push(json!({
                        "role": map_role("system", config),
                        "content": system,
                    }));
                }
            }

            if !request.messages.is_empty() {
                for message in &request.messages {
                    if is_bedrock {
                        messages.push(json!({
                            "role": map_role(&message.role, config),
                            "content": [{"text": message.content}],
                        }));
                    } else {
                        messages.push(json!({
                            "role": map_role(&message.role, config),
                            "content": message.content,
                        }));
                    }
                }
            } else if let Some(user) = &request.user {
                if is_bedrock {
                    messages.push(json!({
                        "role": map_role("user", config),
                        "content": [{"text": user}],
                    }));
                } else if !request.files.is_empty() || !request.images.is_empty() {
                    messages.push(json!({
                        "role": map_role("user", config),
                        "content": build_flat_content_parts(request, config),
                    }));
                } else {
                    messages.push(json!({
                        "role": map_role("user", config),
                        "content": user,
                    }));
                }
            }
            body.insert("messages".into(), Value::Array(messages));
        }
    }
}

fn is_bedrock(config: &ProviderConfig) -> bool {
    config.wraps_options_in == "inferenceConfig" && config.auth_scheme == "SigV4"
}

fn build_flat_content_parts(request: &Request, config: &ProviderConfig) -> Vec<Value> {
    let is_anthropic = matches!(
        system_placement(config.name),
        SystemPlacement::TopLevelField
    );
    let mut parts = Vec::new();

    for file in &request.files {
        if is_anthropic {
            parts.push(json!({
                "type": "document",
                "source": {"type": "file", "file_id": file.id},
            }));
        } else {
            parts.push(json!({
                "type": "file",
                "file": {"file_id": file.id},
            }));
        }
    }

    for image in &request.images {
        if is_anthropic {
            if image.url.starts_with("data:") {
                let (mime_type, data) = parse_data_uri(&image.url);
                parts.push(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": mime_type, "data": data},
                }));
            } else {
                parts.push(json!({
                    "type": "image",
                    "source": {"type": "url", "url": image.url},
                }));
            }
        } else {
            let detail = if image.detail.is_empty() {
                "auto"
            } else {
                &image.detail
            };
            parts.push(json!({
                "type": "image_url",
                "image_url": {"url": image.url, "detail": detail},
            }));
        }
    }

    if let Some(user) = &request.user {
        parts.push(json!({"type": "text", "text": user}));
    }
    parts
}

fn build_google_parts(request: &Request) -> Option<Vec<Value>> {
    if request.files.is_empty() && request.images.is_empty() {
        return request
            .user
            .as_ref()
            .map(|user| vec![json!({"text": user})]);
    }

    let mut parts = Vec::new();
    for file in &request.files {
        parts.push(json!({
            "file_data": {"file_uri": file.uri, "mime_type": file.mime_type}
        }));
    }
    for image in &request.images {
        if image.url.starts_with("data:") {
            let (mime_type, data) = parse_data_uri(&image.url);
            parts.push(json!({
                "inline_data": {"mime_type": mime_type, "data": data}
            }));
        }
    }
    if let Some(user) = &request.user {
        parts.push(json!({"text": user}));
    }
    Some(parts)
}

fn parse_data_uri(uri: &str) -> (String, String) {
    if !uri.starts_with("data:") {
        return (String::new(), uri.to_string());
    }
    let remainder = &uri["data:".len()..];
    let mut parts = remainder.splitn(2, ',');
    let meta = parts.next().unwrap_or_default();
    let data = parts.next().unwrap_or_default();
    (
        meta.trim_end_matches(";base64").to_string(),
        data.to_string(),
    )
}

fn add_options(
    body: &mut Map<String, Value>,
    provider: &Provider,
    model: &str,
    options: &PromptOptions,
) {
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::Temperature,
        options.temperature.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::TopP,
        options.top_p.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::TopK,
        options.top_k.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::Seed,
        options.seed.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::FrequencyPenalty,
        options.frequency_penalty.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::PresencePenalty,
        options.presence_penalty.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::ThinkingBudget,
        options.thinking_budget.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::ReasoningEffort,
        options.reasoning_effort.clone().map(Value::from),
    );
    if !options.stop_sequences.is_empty() {
        maybe_insert(
            body,
            provider,
            model,
            OptionKey::StopSequences,
            Some(Value::Array(
                options
                    .stop_sequences
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            )),
        );
    }
}

fn maybe_insert(
    body: &mut Map<String, Value>,
    provider: &Provider,
    model: &str,
    key: OptionKey,
    value: Option<Value>,
) {
    let Some(value) = value else {
        return;
    };
    if let Some(json_key) = resolve_option_key(provider.name, model, key) {
        insert_nested_field(body, json_key, value);
    }
}

/// Wire (JSON) key for `key` on `(provider, model)`.
///
/// Per-model overrides (ADR-024) outrank the provider default table: an exact
/// ModelID match wins outright, otherwise the longest-prefix glob wins, and
/// failing any override the provider's default supported-options key is used.
pub(crate) fn resolve_option_key(
    provider: crate::ProviderName,
    model: &str,
    key: OptionKey,
) -> Option<&'static str> {
    let mut best_key: Option<&'static str> = None;
    let mut best_len: isize = -1;
    for ov in model_option_overrides(provider) {
        if ov.key != key {
            continue;
        }
        if ov.matcher_kind == "id" {
            if ov.matcher_value == model {
                return Some(ov.json_key);
            }
        } else {
            // "pattern": literal prefix + single trailing '*'
            let prefix = ov
                .matcher_value
                .strip_suffix('*')
                .unwrap_or(ov.matcher_value);
            if model.starts_with(prefix) && prefix.len() as isize > best_len {
                best_key = Some(ov.json_key);
                best_len = prefix.len() as isize;
            }
        }
    }
    if best_len >= 0 {
        return best_key;
    }
    supported_options(provider)
        .iter()
        .find(|option| option.key == key)
        .map(|option| option.json_key)
}

fn insert_nested_field(body: &mut Map<String, Value>, path: &str, value: Value) {
    let mut current = body;
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.insert(part.to_string(), value);
            return;
        }
        let entry = current
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry.as_object_mut().expect("nested option object");
    }
}

fn map_role(role: &str, config: &ProviderConfig) -> String {
    config
        .role_mappings
        .iter()
        .find(|(from, _)| *from == role)
        .map(|(_, to)| (*to).to_string())
        .unwrap_or_else(|| role.to_string())
}

fn add_structured_output(
    body: &mut Map<String, Value>,
    headers: &mut Vec<(String, String)>,
    schema: &str,
    provider_name: crate::ProviderName,
) {
    let Some(def) = structured_output(provider_name) else {
        return;
    };
    let Ok(mut parsed_schema) = serde_json::from_str::<Value>(schema) else {
        return;
    };

    if def.enforce_strict {
        set_additional_properties_false(&mut parsed_schema);
    }
    if def.remove_additional_props {
        remove_additional_properties(&mut parsed_schema);
    }
    if !def.beta_header.is_empty() {
        headers.push(("anthropic-beta".into(), def.beta_header.into()));
    }

    let path_parts: Vec<&str> = def.schema_path.split('.').collect();
    let format_object = if path_parts.len() == 1 {
        json!({
            "type": def.format_type,
            path_parts[0]: parsed_schema,
        })
    } else {
        json!({
            "type": def.format_type,
            path_parts[0]: {
                "name": "response",
                path_parts[1]: parsed_schema,
                "strict": def.enforce_strict,
            }
        })
    };
    insert_nested_field(body, def.format_field, format_object);
}

fn set_additional_properties_false(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if object.get("type").and_then(Value::as_str) == Some("object") {
        object.insert("additionalProperties".into(), Value::Bool(false));
        let required_missing = !object.contains_key("required");
        let required_keys = object
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| {
                properties
                    .keys()
                    .cloned()
                    .map(Value::String)
                    .collect::<Vec<_>>()
            });
        if required_missing {
            if let Some(keys) = required_keys {
                object.insert("required".into(), Value::Array(keys));
            }
        }
        if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
            for value in properties.values_mut() {
                set_additional_properties_false(value);
            }
        }
    }
    if let Some(items) = object.get_mut("items") {
        set_additional_properties_false(items);
    }
}

fn remove_additional_properties(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    object.remove("additionalProperties");
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        for value in properties.values_mut() {
            remove_additional_properties(value);
        }
    }
    if let Some(items) = object.get_mut("items") {
        remove_additional_properties(items);
    }
}
