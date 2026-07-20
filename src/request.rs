use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::options::PromptOptions;
use crate::providers::generated::options::{
    model_option_overrides, option_overrides, supported_options, OptionKey,
};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::request::{
    auth_scheme, file_upload_config, structured_output, system_placement, AuthScheme,
    SystemPlacement,
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

pub fn build_url(provider: &Provider, config: &ProviderSpec) -> String {
    let mut base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| config.base_url.to_string());
    if !config.region_env_var.is_empty() {
        if let Ok(region) = std::env::var(config.region_env_var) {
            base = base.replace("{region}", &region);
        }
    }

    //
    //
    let model = resolve_model(provider, config).unwrap_or_default();
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

///
///
///
///
///
///
///
pub const RESPONSES: &str = "responses";

///
///
fn protocol_wire_shape(token: &str) -> Option<&'static str> {
    if token == RESPONSES {
        Some("ChatResponsesOpenAI")
    } else {
        None
    }
}

///
///
///
///
///
///
pub(crate) fn resolve_chat_protocol(
    config: &ProviderSpec,
    token: &str,
) -> Result<ProviderSpec, Error> {
    if token.is_empty() {
        return Ok(*config);
    }
    let Some(want) = protocol_wire_shape(token) else {
        return Err(Error::Validation {
            field: "protocol",
            message: format!("unknown protocol: {token}"),
        });
    };
    for cp in config.chat_protocols {
        if cp.wire_shape == want {
            let mut resolved = *config;
            resolved.endpoint = cp.endpoint;
            resolved.chat_wire_shape = cp.wire_shape;
            return Ok(resolved);
        }
    }
    Err(Error::Validation {
        field: "protocol",
        message: format!(
            "provider {:?} does not support protocol {:?}",
            config.slug, token
        ),
    })
}

pub fn build_auth_headers(provider: &Provider, config: &ProviderSpec) -> Vec<(String, String)> {
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
    //
    //
    //
    for (k, v) in &provider.headers {
        if !headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(k)) {
            headers.push((k.clone(), v.clone()));
        }
    }
    headers
}

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
///
pub(crate) fn resolve_model(
    provider: &Provider,
    config: &ProviderSpec,
) -> Result<String, Error> {
    if let Some(model) = &provider.model {
        return Ok(model.clone());
    }
    if config.default_model.is_empty() {
        return Err(Error::Validation {
            field: "model",
            message: format!(
                "no model chosen and \"{}\" declares no default; pick one (models().live() lists what the daemon serves)",
                config.slug
            ),
        });
    }
    Ok(config.default_model.to_string())
}

pub(crate) fn build_request(
    provider: &Provider,
    request: &Request,
    msgs: &[crate::transforms::Msg],
    options: &PromptOptions,
    tools: &[crate::Tool],
) -> Result<(Value, Vec<(String, String)>), Error> {
    //
    //
    //
    //
    let config = provider_config(provider.name);
    let config = resolve_chat_protocol(config, options.protocol.as_deref().unwrap_or(""))?;
    let config = &config;

    let model = resolve_model(provider, config)?;

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
                if config.chat_wire_shape == "ChatBedrock" {
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

    crate::transforms::apply_message_shape(&mut body, msgs, request, config);

    //
    //
    if !tools.is_empty() {
        crate::transforms::apply_tool_defs(&mut body, config, tools);
    }

    //
    //
    //
    //
    //
    if !config.wraps_options_in.is_empty() {
        let mut wrapped = Map::new();
        let root_extras = add_options(&mut wrapped, provider, &model, options);
        if let Some(json_key) = resolve_option_key(provider.name, &model, OptionKey::MaxTokens) {
            insert_nested_field(&mut wrapped, json_key, json!(max_tokens));
            body.remove(json_key);
        }
        if !wrapped.is_empty() {
            body.insert(config.wraps_options_in.into(), Value::Object(wrapped));
        }
        deep_merge(&mut body, root_extras);
    } else {
        let root_extras = add_options(&mut body, provider, &model, options);
        deep_merge(&mut body, root_extras);
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

    //
    //
    //
    //
    //
    //
    if !request.files.is_empty() {
        if let Some(upload) = file_upload_config(provider.name) {
            if !upload.beta_header.is_empty() {
                if let Some(entry) = headers
                    .iter_mut()
                    .find(|(k, _)| k.eq_ignore_ascii_case("anthropic-beta"))
                {
                    entry.1 = append_beta(&entry.1, upload.beta_header);
                } else {
                    headers.push(("anthropic-beta".into(), upload.beta_header.into()));
                }
            }
        }
    }

    //
    //
    //
    //
    //
    if config.chat_wire_shape == "ChatResponsesOpenAI" {
        if let Some(value) = body.remove("max_tokens") {
            body.insert("max_output_tokens".into(), value);
        }
    }

    Ok((Value::Object(body), headers))
}

///
///
///
///
fn add_options(
    body: &mut Map<String, Value>,
    provider: &Provider,
    model: &str,
    options: &PromptOptions,
) -> Map<String, Value> {
    let mut root_extras = Map::new();
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::Temperature,
        options.temperature.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::TopP,
        options.top_p.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::TopK,
        options.top_k.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::Seed,
        options.seed.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::FrequencyPenalty,
        options.frequency_penalty.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::PresencePenalty,
        options.presence_penalty.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::ThinkingBudget,
        options.thinking_budget.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        OptionKey::ReasoningEffort,
        options.reasoning_effort.clone().map(Value::from),
        &mut root_extras,
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
            &mut root_extras,
        );
    }
    root_extras
}

fn maybe_insert(
    body: &mut Map<String, Value>,
    provider: &Provider,
    model: &str,
    key: OptionKey,
    value: Option<Value>,
    root_extras: &mut Map<String, Value>,
) {
    let Some(value) = value else {
        return;
    };
    if let Some(json_key) = resolve_option_key(provider.name, model, key) {
        insert_nested_field(body, json_key, value);
        //
        //
        //
        //
        //
        //
        if let Some(ov) = option_overrides(provider.name)
            .iter()
            .find(|entry| entry.key == key && !entry.extra_fields_json.is_empty())
        {
            if let Ok(Value::Object(extras)) =
                serde_json::from_str::<Value>(ov.extra_fields_json)
            {
                merge_into_parent(body, json_key, extras);
            }
        }
        //
        //
        //
        //
        if let Some(ov) = option_overrides(provider.name)
            .iter()
            .find(|entry| entry.key == key && !entry.root_extra_fields_json.is_empty())
        {
            if let Ok(Value::Object(extras)) =
                serde_json::from_str::<Value>(ov.root_extra_fields_json)
            {
                deep_merge(root_extras, extras);
            }
        }
    }
}

///
///
///
///
fn deep_merge(dst: &mut Map<String, Value>, src: Map<String, Value>) {
    for (k, v) in src {
        if let Value::Object(sv) = v {
            if let Some(Value::Object(dv)) = dst.get_mut(&k) {
                deep_merge(dv, sv);
                continue;
            }
            dst.insert(k, Value::Object(sv));
        } else {
            dst.insert(k, v);
        }
    }
}

///
///
fn merge_into_parent(body: &mut Map<String, Value>, path: &str, extras: Map<String, Value>) {
    let mut parts: Vec<&str> = path.split('.').collect();
    parts.pop(); // drop the leaf
    let mut current = body;
    for part in parts {
        let Some(next) = current.get_mut(part).and_then(Value::as_object_mut) else {
            return;
        };
        current = next;
    }
    for (k, v) in extras {
        current.insert(k, v);
    }
}

///
///
///
///
///
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
            //
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

///
///
///
///
pub(crate) fn append_beta(existing: &str, add: &str) -> String {
    if add.is_empty() {
        return existing.to_string();
    }
    if existing.is_empty() {
        return add.to_string();
    }
    if existing.split(',').any(|flag| flag.trim() == add) {
        return existing.to_string();
    }
    format!("{existing},{add}")
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

    //
    //
    //
    if def.schema_placement == "SiblingOfFormat" {
        insert_nested_field(body, def.format_field, json!(def.format_type));
        insert_nested_field(body, def.schema_path, parsed_schema);
        return;
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
