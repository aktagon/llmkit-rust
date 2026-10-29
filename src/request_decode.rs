//! `decode_request`: the inverse of `encode_request` for the ChatOpenAI wire
//! shape (ADR-098 RQC-007..013). Hand-written per SDK, like the encoding
//! transforms (RQC-009); the option keys come from the generated override
//! tables (RQC-008).

use std::collections::{BTreeMap, HashSet};

use serde_json::{Map, Value};

use crate::error::Error;
use crate::providers::generated::options::{
    model_option_overrides, supported_options, wire_shape_option_overrides, OptionKey,
};
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::request::with_wire_shape;
use crate::structs::{Message, Request, WireRequest};
use crate::ProviderName;

fn invalid(field: impl Into<String>, message: impl Into<String>) -> Error {
    Error::Validation {
        field: field.into(),
        message: message.into(),
    }
}

/// Decodes a chat request into a [`Request`], the inverse of
/// [`encode_request`](crate::encode_request) (ADR-098). It is keyless and
/// makes no network call. An empty `chat_wire_shape` selects the provider's
/// default.
///
/// It decodes the ChatOpenAI wire shape only (RQC-011): the model, the turns
/// (a leading system turn becomes `Request.system`) with text content, and
/// every option the provider takes, under any key the encoder may write for
/// it. So `max_tokens` and `max_completion_tokens` both decode to
/// `max_tokens` (RQC-010). The model comes from the body, or from
/// `wire.path` where the provider puts it in the URL. Any field the Request
/// does not model, such as `stream`, `tools` or a content list, fails with
/// [`Error::Validation`] naming it (RQC-007).
pub fn decode_request(
    provider: ProviderName,
    chat_wire_shape: &str,
    wire: &WireRequest,
) -> Result<Request, Error> {
    let mut config = *provider_config(provider);
    if !chat_wire_shape.is_empty() {
        config = with_wire_shape(&config, chat_wire_shape).ok_or_else(|| {
            invalid(
                "chat_wire_shape",
                format!(
                    "provider {:?} does not support wire shape {chat_wire_shape:?}",
                    config.slug
                ),
            )
        })?;
    }
    if config.chat_wire_shape != "ChatOpenAI" {
        return Err(invalid(
            "chat_wire_shape",
            format!(
                "decode_request decodes ChatOpenAI only, not {:?}",
                config.chat_wire_shape
            ),
        ));
    }

    let mut body = match serde_json::from_slice::<Value>(&wire.body) {
        Ok(Value::Object(body)) => body,
        _ => return Err(invalid("body", "must be a JSON object")),
    };

    let mut request = Request::default();
    decode_model(&mut request, &config, &mut body, &wire.path)?;
    decode_options(&mut request, provider, &config, &mut body)?;
    let turns = body.remove("messages");
    reject_unmodeled(&body, "")?;
    if let Some(turns) = turns {
        decode_turns(&mut request, &config, turns)?;
    }
    Ok(request)
}

/// Reads the model from the body, or from the path where the provider's
/// endpoint carries it (`{model}`, as `encode_request` fills it in).
fn decode_model(
    request: &mut Request,
    config: &ProviderSpec,
    body: &mut Map<String, Value>,
    path: &str,
) -> Result<(), Error> {
    if !config.model_in_body {
        let Some((prefix, suffix)) = config.endpoint.split_once("{model}") else {
            return Ok(());
        };
        if path.len() <= prefix.len() + suffix.len()
            || !path.starts_with(prefix)
            || !path.ends_with(suffix)
        {
            return Err(invalid("path", format!("must match {}", config.endpoint)));
        }
        request.model = path[prefix.len()..path.len() - suffix.len()].to_string();
        return Ok(());
    }
    match body.remove("model") {
        None => Ok(()),
        Some(Value::String(model)) => {
            request.model = model;
            Ok(())
        }
        Some(_) => Err(invalid("model", "must be a string")),
    }
}

/// Maps every wire key the encoder may write for an option of (provider, wire
/// shape) to that option (RQC-008): the wire shape's key, which replaces the
/// others for every model (BUG-075), else the provider's key and each
/// per-model override key (ADR-024). CQ-OPT-011 checks that no key maps to two
/// options. A `BTreeMap`, so the keys are read in order in every SDK.
fn decode_option_keys(
    provider: ProviderName,
    chat_wire_shape: &str,
) -> BTreeMap<&'static str, OptionKey> {
    let shape_keys = wire_shape_option_overrides(chat_wire_shape);
    let shape_key = |key: OptionKey| shape_keys.iter().find(|(k, _)| *k == key).map(|(_, j)| *j);
    let supported = supported_options(provider);
    let mut keys = BTreeMap::new();
    for option in supported {
        keys.insert(shape_key(option.key).unwrap_or(option.json_key), option.key);
    }
    for ov in model_option_overrides(provider) {
        if supported.iter().any(|o| o.key == ov.key) && shape_key(ov.key).is_none() {
            keys.insert(ov.json_key, ov.key);
        }
    }
    keys
}

/// Takes each option out of the body, or out of the provider's options
/// container (`wraps_options_in`), into its Request field (RQC-010).
fn decode_options(
    request: &mut Request,
    provider: ProviderName,
    config: &ProviderSpec,
    body: &mut Map<String, Value>,
) -> Result<(), Error> {
    let wrap = config.wraps_options_in;
    let mut wrapped;
    let (container, prefix) = if wrap.is_empty() {
        (body, String::new())
    } else {
        wrapped = match body.remove(wrap) {
            None => return Ok(()),
            Some(Value::Object(inner)) => inner,
            Some(_) => return Err(invalid(wrap, "must be an object")),
        };
        (&mut wrapped, format!("{wrap}."))
    };
    let mut seen = HashSet::new();
    for (key, param) in decode_option_keys(provider, config.chat_wire_shape) {
        let Some(value) = take_nested(container, key) else {
            continue;
        };
        if !seen.insert(param) {
            return Err(invalid(
                format!("{prefix}{key}"),
                format!("sets {param:?} a second time"),
            ));
        }
        set_option(request, param, &value).map_err(|m| invalid(format!("{prefix}{key}"), m))?;
    }
    if !wrap.is_empty() {
        reject_unmodeled(container, &prefix)?;
    }
    Ok(())
}

/// Stores one decoded option value in its Request field, the inverse of
/// `add_options`. Returns the problem with the value, if any.
fn set_option(request: &mut Request, param: OptionKey, value: &Value) -> Result<(), &'static str> {
    let integer = || value.as_i64().ok_or("must be an integer");
    let number = || value.as_f64().ok_or("must be a number");
    match param {
        OptionKey::MaxTokens => request.max_tokens = Some(integer()?),
        OptionKey::TopK => request.top_k = Some(integer()?),
        OptionKey::ThinkingBudget => request.thinking_budget = Some(integer()?),
        OptionKey::Seed => request.seed = Some(integer()?),
        OptionKey::Temperature => request.temperature = Some(number()?),
        OptionKey::TopP => request.top_p = Some(number()?),
        OptionKey::FrequencyPenalty => request.frequency_penalty = Some(number()?),
        OptionKey::PresencePenalty => request.presence_penalty = Some(number()?),
        OptionKey::ReasoningEffort => {
            request.reasoning_effort = Some(value.as_str().ok_or("must be a string")?.to_string())
        }
        OptionKey::StopSequences => {
            request.stop = value
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .map(|s| s.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or("must be a list of strings")?
        }
    }
    Ok(())
}

/// Removes the value at a dotted key (`a.b`) from `map` and returns it. A
/// parent object left empty is removed too, so it does not read as an
/// unmodeled field.
fn take_nested(map: &mut Map<String, Value>, key: &str) -> Option<Value> {
    let Some((head, rest)) = key.split_once('.') else {
        return map.remove(key);
    };
    let child = map.get_mut(head)?.as_object_mut()?;
    let value = take_nested(child, rest)?;
    if child.is_empty() {
        map.remove(head);
    }
    Some(value)
}

/// Fails on the first field left in `map`, in key order, so every SDK names
/// the same field (RQC-007).
fn reject_unmodeled(map: &Map<String, Value>, prefix: &str) -> Result<(), Error> {
    match map.keys().min() {
        Some(key) => Err(invalid(
            format!("{prefix}{key}"),
            "is not a field decode_request decodes",
        )),
        None => Ok(()),
    }
}

/// Reads the messages array: a leading system turn into `Request.system`,
/// then user and assistant turns with text content. Roles are read back
/// through the provider's role mappings.
fn decode_turns(request: &mut Request, config: &ProviderSpec, turns: Value) -> Result<(), Error> {
    let Value::Array(turns) = turns else {
        return Err(invalid("messages", "must be a list"));
    };
    for (i, turn) in turns.into_iter().enumerate() {
        let field = format!("messages[{i}]");
        let Value::Object(mut turn) = turn else {
            return Err(invalid(field, "must be an object"));
        };
        let role = turn.remove("role");
        let content = turn.remove("content");
        reject_unmodeled(&turn, &format!("{field}."))?;
        let Some(Value::String(role)) = role else {
            return Err(invalid(format!("{field}.role"), "must be a string"));
        };
        let Some(Value::String(content)) = content else {
            return Err(invalid(
                format!("{field}.content"),
                "only text content is decoded",
            ));
        };
        let canonical = config
            .role_mappings
            .iter()
            .find(|(_, on_wire)| *on_wire == role)
            .map(|(canonical, _)| *canonical);
        match canonical {
            Some("system") => {
                if i != 0 || config.system_placement != "MessageInArray" {
                    return Err(invalid(
                        format!("{field}.role"),
                        "a system turn is decoded only as the first message",
                    ));
                }
                request.system = content;
            }
            Some(role @ ("user" | "assistant")) => {
                request.messages.push(Message::new(role, content))
            }
            _ => {
                return Err(invalid(
                    format!("{field}.role"),
                    format!("role {role:?} is not decoded"),
                ))
            }
        }
    }
    Ok(())
}
