use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::providers::generated::options::{
    model_option_overrides, option_overrides, supported_options, wire_shape_option_overrides,
    wire_shape_required_params, OptionKey,
};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::request::{
    auth_scheme, file_upload_config, structured_output, system_placement, AuthScheme,
    SystemPlacement,
};
use crate::structs::{Request, WireRequest};
use crate::types::Provider;
use crate::ProviderName;

pub fn validate_provider(provider: &Provider) -> Result<(), Error> {
    if provider.api_key.is_empty() {
        return Err(Error::Validation {
            field: "api_key".into(),
            message: "required".into(),
        });
    }
    Ok(())
}

/// Checks that the request carries a turn. The final prompt is a turn like any
/// other (ADR-098), so empty `messages` means nothing to send.
pub fn validate_request(request: &Request) -> Result<(), Error> {
    if request.messages.is_empty() {
        return Err(Error::Validation {
            field: "messages".into(),
            message: "set text(), history() or image() before calling prompt".into(),
        });
    }
    Ok(())
}

/// Checks that the provider takes each option the request sets.
pub fn validate_options(provider: ProviderName, options: &Request) -> Result<(), Error> {
    let supported = supported_options(provider);

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
        if let Some(override_def) = option_overrides(provider).iter().find(|entry| {
            entry.key == OptionKey::ReasoningEffort && !entry.allowed_values.is_empty()
        }) {
            if !override_def.allowed_values.contains(&value) {
                return Err(Error::Validation {
                    field: "reasoning_effort".into(),
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
    provider: ProviderName,
    supported: &[crate::SupportedOptionDef],
    key: OptionKey,
    field: &'static str,
) -> Result<(), Error> {
    if enabled && !supported.iter().any(|option| option.key == key) {
        return Err(Error::Validation {
            field: field.into(),
            message: format!("not supported by {provider:?}"),
        });
    }
    Ok(())
}

/// The endpoint with the model filled in where the wire shape puts it
/// (`WireRequest.path`, RQC-006). It holds no key.
pub(crate) fn request_path(config: &ProviderSpec, model: &str) -> String {
    config.endpoint.replace("{model}", model)
}

/// Turns a keyless request path into the URL the send path posts to: the
/// client's base URL, the region, and the key where the provider takes it in
/// the URL (Google's query parameter, an `{apiKey}` template).
pub(crate) fn send_url(provider: &Provider, config: &ProviderSpec, path: &str) -> String {
    let mut base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| config.base_url.to_string());
    if !config.region_env_var.is_empty() {
        if let Ok(region) = std::env::var(config.region_env_var) {
            base = base.replace("{region}", &region);
        }
    }

    let mut endpoint = path.replace("{apiKey}", &provider.api_key);

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

/// ADR-055 opt-in chat-protocol token for OpenAI's Responses API. Pass it to
/// `Text::protocol` to POST the `{input}` envelope to `/v1/responses` instead
/// of the default Chat Completions `{messages}` envelope to
/// `/v1/chat/completions`. It is a plain string; `text().protocol("responses")`
/// is equivalent (per-SDK idiom note, ADR-055 — Rust keeps the string token
/// rather than a `Protocol::Responses` enum, mirroring the builder's
/// `impl Into<String>` chain method).
pub const RESPONSES: &str = "responses";

/// Maps a public protocol token to its `ChatWireShape` local name. An
/// unknown token yields `None`.
fn protocol_wire_shape(token: &str) -> Option<&'static str> {
    if token == RESPONSES {
        Some("ChatResponsesOpenAI")
    } else {
        None
    }
}

/// Returns `config` with `endpoint` + `chat_wire_shape` overridden for a
/// non-default chat protocol opt-in (ADR-055 `Protocol(...)`). An empty token
/// keeps the default (config unchanged). A provider that does not expose the
/// requested protocol raises `Error::Validation { field: "protocol", .. }` —
/// the loud, uniform error the ADR requires — before any network call.
/// `ProviderSpec` is `Copy`, so the override never leaks to other calls.
pub(crate) fn resolve_chat_protocol(
    config: &ProviderSpec,
    token: &str,
) -> Result<ProviderSpec, Error> {
    if token.is_empty() {
        return Ok(*config);
    }
    let Some(want) = protocol_wire_shape(token) else {
        return Err(Error::Validation {
            field: "protocol".into(),
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
        field: "protocol".into(),
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
    // ADR-052: add caller custom headers that do not collide with the auth
    // or required header (those always win), so a gateway header (e.g.
    // cf-aig-authorization) rides alongside the provider key.
    for (k, v) in &provider.headers {
        if !headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(k)) {
            headers.push((k.clone(), v.clone()));
        }
    }
    headers
}

/// Adds the send path's headers to the content headers: the provider auth
/// header, then the caller's headers (`Client::add_header`, ADR-052), which
/// never clobber the auth or protocol headers (header names are
/// case-insensitive). A gateway header (cf-aig-authorization) still rides
/// alongside the provider key.
pub(crate) fn send_headers(
    provider: &Provider,
    config: &ProviderSpec,
    content: &[(String, String)],
) -> Vec<(String, String)> {
    let mut headers = content.to_vec();
    match auth_scheme(provider.name) {
        AuthScheme::BearerToken => headers.push((
            config.auth_header.to_string(),
            format!("{} {}", config.auth_prefix, provider.api_key),
        )),
        AuthScheme::HeaderApiKey => {
            headers.push((config.auth_header.to_string(), provider.api_key.clone()))
        }
        AuthScheme::QueryParamKey | AuthScheme::SigV4 => {}
    }
    for (k, v) in &provider.headers {
        if !headers.iter().any(|(hk, _)| hk.eq_ignore_ascii_case(k)) {
            headers.push((k.clone(), v.clone()));
        }
    }
    headers
}

/// Encodes a chat request for a provider's wire shape (ADR-098). It is keyless
/// and makes no network call: the [`WireRequest`] carries the path and the
/// content headers, and the caller's send path adds the base URL, the key and
/// its own headers (RQC-006). An empty `chat_wire_shape` selects the
/// provider's default. The model is `request.model`, else the provider's
/// default.
pub fn encode_request(
    provider: ProviderName,
    chat_wire_shape: &str,
    request: &Request,
) -> Result<WireRequest, Error> {
    let config = served_chat_spec(provider, chat_wire_shape)?;
    let chat = encode_chat(&Provider::new(provider, ""), &config, request)?;
    Ok(WireRequest {
        path: request_path(&config, &chat.request.model),
        headers: chat.headers.into_iter().collect(),
        body: crate::json_text::write_json(&chat.body)?.into_bytes(),
    })
}

/// A chat request that passed every check: the prepared Request, its body and
/// its content headers.
pub(crate) struct ChatRequest {
    pub(crate) request: Request,
    pub(crate) body: Value,
    pub(crate) headers: Vec<(String, String)>,
}

/// The one validation step of a chat call: [`prepare_request`], then
/// [`encode_body`] over [`crate::transforms::request_msgs`], which checks
/// every part. Text, stream, the Agent (each turn), batch (each item) and
/// [`encode_request`] all run it before anything else, so a request that
/// fails a check never reaches middleware (HANDOFF-105 F11, owner decision 1).
pub(crate) fn encode_chat(
    provider: &Provider,
    config: &ProviderSpec,
    request: &Request,
) -> Result<ChatRequest, Error> {
    let request = prepare_request(provider, config, request)?;
    let msgs = crate::transforms::request_msgs(&request)?;
    let (body, headers) = encode_body(provider.name, config, &request, &msgs)?;
    Ok(ChatRequest {
        request,
        body,
        headers,
    })
}

/// The one served-shape check: [`encode_request`], `decode_response` and
/// `encode_response` all run it (HANDOFF-105 F11). It returns the provider's
/// config with the endpoint and wire shape of `chat_wire_shape`, or of the
/// provider's default when it is "". A shape the provider does not serve is
/// `Error::Validation` on field `chat_wire_shape` (owner decision 2,
/// HANDOFF-105 section 3b), and so is a provider that serves no chat shape at
/// all: writing provider paths into another family's body produces a body
/// neither family reads.
pub(crate) fn served_chat_spec(
    provider: ProviderName,
    chat_wire_shape: &str,
) -> Result<ProviderSpec, Error> {
    let config = provider_config(provider);
    let shape = if chat_wire_shape.is_empty() {
        config.chat_wire_shape
    } else {
        chat_wire_shape
    };
    match with_wire_shape(config, shape) {
        Some(served) if !served.chat_wire_shape.is_empty() => Ok(served),
        _ => Err(Error::Validation {
            field: "chat_wire_shape".into(),
            message: format!(
                "provider {:?} does not serve chat wire shape {chat_wire_shape:?}",
                config.slug
            ),
        }),
    }
}

/// `config` with the endpoint and wire shape of one of the provider's chat
/// protocols. The default shape is `config` itself.
pub(crate) fn with_wire_shape(config: &ProviderSpec, shape: &str) -> Option<ProviderSpec> {
    if shape == config.chat_wire_shape {
        return Some(*config);
    }
    config
        .chat_protocols
        .iter()
        .find(|cp| cp.wire_shape == shape)
        .map(|cp| {
            let mut resolved = *config;
            resolved.endpoint = cp.endpoint;
            resolved.chat_wire_shape = cp.wire_shape;
            resolved
        })
}

/// The one setup step of a chat call (RQC-017): Text, stream, the Agent, batch
/// and [`encode_request`] all run it before they encode. It checks that the
/// request has a turn and that the provider takes each option, then resolves
/// the model: `request.model`, which carries the builder's model, wins over
/// `provider.model` and the provider's default (RQC-004). The returned
/// Request carries the resolved model.
pub(crate) fn prepare_request(
    provider: &Provider,
    config: &ProviderSpec,
    request: &Request,
) -> Result<Request, Error> {
    validate_request(request)?;
    validate_options(provider.name, request)?;
    let mut request = request.clone();
    if request.model.is_empty() {
        request.model = resolve_model(provider, config)?;
    }
    Ok(request)
}

/// ADR-031 honest no-default contract: the single predicate every model
/// resolution point dispatches on. Local daemons declare no default — what a
/// daemon serves is runtime inventory, not a registry fact — so both empty
/// is a validation error instead of a guess the daemon may not have pulled.
pub(crate) fn resolve_model(provider: &Provider, config: &ProviderSpec) -> Result<String, Error> {
    if let Some(model) = &provider.model {
        return Ok(model.clone());
    }
    if config.default_model.is_empty() {
        return Err(Error::Validation {
            field: "model".into(),
            message: format!(
                "no model chosen and \"{}\" declares no default; pick one (models().list() lists what the daemon serves)",
                config.slug
            ),
        });
    }
    Ok(config.default_model.to_string())
}

/// The one chat body builder (RQC-001): [`encode_request`], Text, stream,
/// batch and the Agent loop all encode through it. It returns the body as a
/// value, because caching, streaming and the batch envelope change it after
/// encoding, and the content headers. `request.model` must be resolved
/// ([`prepare_request`]); `config` carries the effective wire shape.
///
/// `msgs` is the internal message sum (ADR-026 PIPE-007) and the only
/// conversation input the message transforms read: `request.messages`
/// through [`crate::transforms::request_msgs`], validated at the part-kind
/// boundary (PIPE-008, ADR-100 MSG-002).
///
/// Text, batch and stream set no tools, so the tool-def step is a no-op there
/// and their wire body stays byte-identical (ADR-026 PIPE-005).
pub(crate) fn encode_body(
    provider: ProviderName,
    config: &ProviderSpec,
    request: &Request,
    msgs: &[crate::transforms::Msg],
) -> Result<(Value, Vec<(String, String)>), Error> {
    let model = request.model.clone();

    let mut body = Map::new();
    let mut headers: Vec<(String, String)> = Vec::new();

    if config.model_in_body {
        body.insert("model".into(), Value::String(model.clone()));
    }

    // The caller's value, else the provider's default where the wire shape
    // requires the param, else none (ADR-098 RQC-015).
    let max_tokens = request.max_tokens.or_else(|| {
        wire_shape_required_params(config.chat_wire_shape)
            .contains(&OptionKey::MaxTokens)
            .then(|| i64::from(config.default_max_tokens))
    });
    if let (Some(max_tokens), Some(json_key)) = (
        max_tokens,
        resolve_option_key(
            provider,
            &model,
            config.chat_wire_shape,
            OptionKey::MaxTokens,
        ),
    ) {
        body.insert(json_key.to_string(), json!(max_tokens));
    }

    match system_placement(provider) {
        SystemPlacement::TopLevelField => {
            if !request.system.is_empty() {
                if config.chat_wire_shape == "ChatBedrock" {
                    body.insert("system".into(), json!([{ "text": request.system }]));
                } else {
                    body.insert("system".into(), Value::String(request.system.clone()));
                }
            }
        }
        SystemPlacement::MessageInArray => {}
        SystemPlacement::SiblingObject => {
            if !request.system.is_empty() {
                body.insert(
                    "system_instruction".into(),
                    json!({"parts": [{"text": request.system}]}),
                );
            }
        }
    }

    // resolve_turns runs first and only here: it is the one place the config and
    // the message list meet, so the ADR-085 RSN-006 shape check is made once
    // rather than remembered in each transform.
    crate::transforms::check_request_tool_calls(msgs, config)?;
    let msgs = crate::provider_turn::resolve_turns(msgs, config);
    crate::transforms::apply_message_shape(&mut body, &msgs, request, config);

    // Tool definitions (Agent path). An empty tool slice on Text/batch/stream
    // is a no-op, so their wire body stays byte-identical (PIPE-005).
    if !request.tools.is_empty() {
        crate::transforms::apply_tool_defs(&mut body, config, &request.tools);
    }

    // Root extras (ADR-029 THK-003): options whose override carries
    // root_extra_fields_json imply a sibling object at the body ROOT (e.g.
    // {"thinking":{"type":"adaptive"}} alongside Anthropic's
    // output_config.effort). add_options collects them; they deep-merge into
    // the true body root below — never into the wraps_options_in wrapper.
    if !config.wraps_options_in.is_empty() {
        let mut wrapped = Map::new();
        let root_extras = add_options(
            &mut wrapped,
            provider,
            &model,
            config.chat_wire_shape,
            request,
        );
        if let (Some(max_tokens), Some(json_key)) = (
            max_tokens,
            resolve_option_key(
                provider,
                &model,
                config.chat_wire_shape,
                OptionKey::MaxTokens,
            ),
        ) {
            insert_nested_field(&mut wrapped, json_key, json!(max_tokens));
            body.remove(json_key);
        }
        if !wrapped.is_empty() {
            body.insert(config.wraps_options_in.into(), Value::Object(wrapped));
        }
        deep_merge(&mut body, root_extras);
    } else {
        let root_extras = add_options(&mut body, provider, &model, config.chat_wire_shape, request);
        deep_merge(&mut body, root_extras);
    }

    if !config.safety_settings_wire_path.is_empty() && !request.safety_settings.is_empty() {
        let ss: Vec<Value> = request
            .safety_settings
            .iter()
            .map(|s| json!({"category": s.category, "threshold": s.threshold}))
            .collect();
        body.insert(
            config.safety_settings_wire_path.to_string(),
            Value::Array(ss),
        );
    }

    if !request.schema.is_empty() {
        add_structured_output(&mut body, &mut headers, &request.schema, provider);
    }

    // BUG-017: a text request that references an uploaded file carries the
    // beta the provider's upload declares (uploadBetaHeader). Anthropic
    // declares none since its Files API left beta (BUG-078).
    // Compose it with any existing anthropic-beta (e.g. the structured-output
    // beta pushed just above) rather than overwriting — comma-separated, deduped.
    if crate::transforms::request_has_files(request) {
        if let Some(upload) = file_upload_config(provider) {
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

    // Required protocol header (e.g. anthropic-version): keyless, so it
    // belongs to the content headers.
    if !config.required_header.is_empty() {
        headers.push((
            config.required_header.to_string(),
            config.required_header_value.to_string(),
        ));
    }

    Ok((Value::Object(body), headers))
}

/// Applies generation parameters to `body` and returns the accumulated root
/// extras (ADR-029 THK-003) for the caller to deep-merge at the body root —
/// returned rather than merged in place because the unwrapped path would
/// otherwise need two mutable borrows of the same map.
fn add_options(
    body: &mut Map<String, Value>,
    provider: ProviderName,
    model: &str,
    chat_wire_shape: &str,
    options: &Request,
) -> Map<String, Value> {
    let mut root_extras = Map::new();
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::Temperature,
        options.temperature.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::TopP,
        options.top_p.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::TopK,
        options.top_k.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::Seed,
        options.seed.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::FrequencyPenalty,
        options.frequency_penalty.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::PresencePenalty,
        options.presence_penalty.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::ThinkingBudget,
        options.thinking_budget.map(Value::from),
        &mut root_extras,
    );
    maybe_insert(
        body,
        provider,
        model,
        chat_wire_shape,
        OptionKey::ReasoningEffort,
        options.reasoning_effort.clone().map(Value::from),
        &mut root_extras,
    );
    if !options.stop.is_empty() {
        maybe_insert(
            body,
            provider,
            model,
            chat_wire_shape,
            OptionKey::StopSequences,
            Some(Value::Array(
                options.stop.iter().cloned().map(Value::String).collect(),
            )),
            &mut root_extras,
        );
    }
    root_extras
}

fn maybe_insert(
    body: &mut Map<String, Value>,
    provider: ProviderName,
    model: &str,
    chat_wire_shape: &str,
    key: OptionKey,
    value: Option<Value>,
    root_extras: &mut Map<String, Value>,
) {
    let Some(value) = value else {
        return;
    };
    if let Some(json_key) = resolve_option_key(provider, model, chat_wire_shape, key) {
        insert_nested_field(body, json_key, value);
        // Static sibling fields from the option override (e.g. Anthropic's
        // {"type":"enabled"} alongside thinking.budget_tokens) merge into the
        // leaf's parent object — mirrors Go addOptions/mergeIntoParent.
        // Caught by the options-anthropic wire fixture (ADR-028 M2): without
        // this merge the thinking object ships without its type field and
        // Anthropic rejects the request.
        if let Some(ov) = option_overrides(provider)
            .iter()
            .find(|entry| entry.key == key && !entry.extra_fields_json.is_empty())
        {
            if let Ok(Value::Object(extras)) = serde_json::from_str::<Value>(ov.extra_fields_json) {
                merge_into_parent(body, json_key, extras);
            }
        }
        // Root extras (ADR-029 THK-003): static fields the option implies at
        // the request body ROOT (e.g. {"thinking":{"type":"adaptive"}}
        // alongside Anthropic's output_config.effort). Accumulated here; the
        // encode_body caller deep-merges them into the true body root.
        if let Some(ov) = option_overrides(provider)
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

/// Deep-merges `src` into `dst`: when both sides hold an object at the same
/// key the objects merge, otherwise `src` overwrites. Used for
/// `root_extra_fields_json` (ADR-029) so e.g. {"thinking":{"type":"adaptive"}}
/// composes with an existing thinking object rather than replacing it.
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

/// Merges `extras` into the object containing the leaf of `path`: for
/// "a.b.c" they land in body["a"]["b"]; for a top-level path, in body.
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

/// Wire (JSON) key for `key` on `(provider, model)` under the effective chat
/// wire shape.
///
/// A wire-shape key (BUG-075) outranks everything: the Responses shape names
/// MaxTokens `max_output_tokens` for every model. Next, per-model overrides
/// (ADR-024) outrank the provider default table: an exact ModelID match wins
/// outright, otherwise the longest-prefix glob wins, and failing any override
/// the provider's default supported-options key is used.
pub(crate) fn resolve_option_key(
    provider: crate::ProviderName,
    model: &str,
    chat_wire_shape: &str,
    key: OptionKey,
) -> Option<&'static str> {
    if let Some((_, json_key)) = wire_shape_option_overrides(chat_wire_shape)
        .iter()
        .find(|(k, _)| *k == key)
    {
        return Some(json_key);
    }
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

/// Composes an `anthropic-beta` header value: appends `add` to `existing`
/// comma-separated, deduping (never overwriting an already-present flag).
/// pub(crate) so the batch submit path can compose the per-request betas onto
/// the batch CREATE request.
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

    // SiblingOfFormat placement (Google): the format field carries the literal
    // format type (responseMimeType: "application/json") and the schema is an
    // independent sibling at schema_path (responseSchema), not nested in a wrapper.
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

// HANDOFF-105 R1-rust: tests of the one setup step (ADR-098 RQC-004,
// RQC-017). They drive the internal Agent and batch paths with a provider
// model and a Request model, which the public builders cannot express.
// Native only: the mock server is a std TCP listener on a thread.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod setup_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use serde_json::Value;

    use crate::middleware::{Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase};
    use crate::options::SendOptions;
    use crate::structs::{Message, Request};
    use crate::types::Provider;
    use crate::ProviderName;

    /// Answers `replies.len()` requests in order and records each JSON body.
    fn recording_server(replies: Vec<&'static str>) -> (String, Arc<Mutex<Vec<Value>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&bodies);
        thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().expect("accept");
                let body = read_body(&mut stream);
                seen.lock()
                    .unwrap()
                    .push(serde_json::from_str(&body).unwrap_or(Value::Null));
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                stream.write_all(response.as_bytes()).expect("respond");
            }
        });
        (format!("http://{addr}"), bodies)
    }

    fn read_body(stream: &mut std::net::TcpStream) -> String {
        let mut data = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).expect("read");
            if n == 0 {
                break;
            }
            data.extend_from_slice(&chunk[..n]);
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&data[..pos]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if data.len() >= pos + 4 + length {
                    return String::from_utf8_lossy(&data[pos + 4..pos + 4 + length]).into_owned();
                }
            }
        }
        String::new()
    }

    /// A middleware that records the model of every pre event of `op`.
    fn record_models(op: MiddlewareOp) -> (MiddlewareFn, Arc<Mutex<Vec<String>>>) {
        let models = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&models);
        let hook: MiddlewareFn = Arc::new(move |event: &Event| {
            if event.op == op && event.phase == MiddlewarePhase::Pre {
                seen.lock().unwrap().push(event.model.clone());
            }
            None
        });
        (hook, models)
    }

    fn provider(name: ProviderName, key: &str, base_url: &str, model: &str) -> Provider {
        let mut p = Provider::new(name, key);
        p.base_url = Some(base_url.into());
        p.model = Some(model.into());
        p
    }

    // Request.model wins over the provider's model in the Agent: the body and the
    // llm_request event both carry it (RQC-004).
    #[tokio::test]
    async fn request_model_wins_in_the_agent() {
        let (url, bodies) =
            recording_server(vec![r#"{"choices":[{"message":{"content":"Helsinki"}}]}"#]);
        let (hook, models) = record_models(MiddlewareOp::LlmRequest);
        let mut agent = crate::agent::Agent::new(
            provider(ProviderName::OpenAI, "sk-test", &url, "gpt-4o"),
            Request {
                model: "gpt-4o-mini".into(),
                ..Request::default()
            },
            Vec::new(),
            SendOptions {
                middleware: vec![hook],
                ..SendOptions::default()
            },
        );
        agent
            .chat("What is the capital of Finland?")
            .await
            .expect("chat");
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0]["model"], "gpt-4o-mini");
        assert_eq!(*models.lock().unwrap(), vec!["gpt-4o-mini".to_string()]);
    }

    // Request.model wins over the provider's model in batch: each item carries it,
    // and the batch_submit event reports it.
    #[tokio::test]
    async fn request_model_wins_in_batch() {
        let (url, bodies) = recording_server(vec![
            r#"{"id":"msgbatch_01","processing_status":"in_progress"}"#,
        ]);
        let (hook, models) = record_models(MiddlewareOp::BatchSubmit);
        crate::batch::submit_batch(
            &provider(
                ProviderName::Anthropic,
                "sk-ant-test",
                &url,
                "claude-opus-4-7",
            ),
            &[Request {
                model: "claude-sonnet-4-6".into(),
                messages: vec![Message::new("user", "What is the capital of Finland?")],
                ..Request::default()
            }],
            SendOptions {
                middleware: vec![hook],
                ..SendOptions::default()
            },
        )
        .await
        .expect("submit_batch");
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0]["requests"][0]["params"]["model"],
            "claude-sonnet-4-6"
        );
        assert_eq!(
            *models.lock().unwrap(),
            vec!["claude-sonnet-4-6".to_string()]
        );
    }
}
