//! `decode_request`: the inverse of `encode_request` for the ChatOpenAI wire
//! shape (ADR-098 RQC-007..013). Hand-written per SDK, like the encoding
//! transforms (RQC-009); the option keys come from the generated override
//! tables (RQC-008).

use std::collections::{BTreeMap, HashSet};

use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::providers::generated::options::{
    model_option_overrides, supported_options, wire_shape_option_overrides, OptionKey,
};
use crate::providers::generated::providers::{provider_config, ProviderSpec, ALL_PROVIDER_NAMES};
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
/// every option some provider of the wire shape writes, under any key the
/// encoder may write for it (owner decision 5, HANDOFF-105 F4). So
/// `max_tokens` and `max_completion_tokens` both decode to `max_tokens`
/// (RQC-010), and `top_k` decodes for OpenAI, which does not take it:
/// `encode_request` checks support (RQC-017). A bare-string `stop` decodes as a one-item list,
/// `stream: false` as no stream, and a `developer` turn as the system turn
/// where the provider declares that role (OpenAI, OpenRouter).
/// The model comes from the body, or from `wire.path` where the provider puts
/// it in the URL. Any field the Request
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
    decode_body(&config, wire)
}

/// Decodes the body (and the path) of a ChatOpenAI request for `config`.
fn decode_body(config: &ProviderSpec, wire: &WireRequest) -> Result<Request, Error> {
    let raw = match serde_json::from_slice::<Box<RawValue>>(&wire.body) {
        Ok(raw) if raw.get().starts_with('{') => raw,
        _ => return Err(invalid("body", "must be a JSON object")),
    };
    let mut numbers = NumberTexts::new();
    let mut body = match parse_fields(&raw, "", Some(&[]), &mut numbers)? {
        Value::Object(body) => body,
        _ => unreachable!("a raw value that starts with {{ parses as an object"),
    };

    let mut request = Request::default();
    decode_model(&mut request, config, &mut body, &wire.path)?;
    decode_options(&mut request, config, &mut body, &numbers)?;
    let turns = body.remove("messages");
    // stream false asks for what an absent stream does (HANDOFF-105 R3b).
    if body.get("stream") == Some(&Value::Bool(false)) {
        body.remove("stream");
    }
    reject_unmodeled(&body, "")?;
    if let Some(turns) = turns {
        decode_turns(&mut request, config, turns)?;
    }
    Ok(request)
}

/// Parses checked JSON one field at a time, so a number serde_json cannot hold
/// (`1e999` overflows f64) fails on its own field, as in Go and TS, instead
/// of failing the whole body (HANDOFF-105 F3). `field` is the path so far, in
/// the decoder's form (`messages[0].content`). The raw value is already valid
/// JSON, so a scalar that does not parse is a number out of range.
///
/// It also keeps each number's text, which an f64 does not hold exactly past
/// 2^53, in `numbers`, by `keys`: the object keys from the body down. A
/// number inside a list is no option, so `keys` is `None` below a list.
fn parse_fields(
    raw: &RawValue,
    field: &str,
    keys: Option<&[String]>,
    numbers: &mut NumberTexts,
) -> Result<Value, Error> {
    let nested = |key: &str| {
        if field.is_empty() {
            key.to_string()
        } else {
            format!("{field}.{key}")
        }
    };
    let malformed = |_| invalid(field, "is not valid JSON");
    match raw.get().as_bytes().first() {
        Some(b'{') => {
            let members: BTreeMap<String, Box<RawValue>> =
                serde_json::from_str(raw.get()).map_err(malformed)?;
            let mut object = Map::new();
            for (key, value) in members {
                let child = keys.map(|keys| [keys, std::slice::from_ref(&key)].concat());
                let value = parse_fields(&value, &nested(&key), child.as_deref(), numbers)?;
                object.insert(key, value);
            }
            Ok(Value::Object(object))
        }
        Some(b'[') => {
            let items: Vec<Box<RawValue>> = serde_json::from_str(raw.get()).map_err(malformed)?;
            let items = items
                .iter()
                .enumerate()
                .map(|(i, item)| parse_fields(item, &format!("{field}[{i}]"), None, numbers))
                .collect::<Result<_, _>>()?;
            Ok(Value::Array(items))
        }
        _ => {
            let value: Value =
                serde_json::from_str(raw.get()).map_err(|_| invalid(field, "is out of range"))?;
            if let (Value::Number(_), Some(keys)) = (&value, keys) {
                numbers.insert(keys.to_vec(), raw.get().to_string());
            }
            Ok(value)
        }
    }
}

/// The text of each number in a body, by its object keys from the body down.
type NumberTexts = BTreeMap<Vec<String>, String>;

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

/// Maps every wire key some provider of the wire shape may write for an
/// option to that option (owner decision 5, HANDOFF-105 F4). decode_request
/// reads what the client sent, which need not be what the named provider
/// takes; the target's encoder checks support (RQC-017). CQ-OPT-011 checks
/// that no key maps to two options on one shape. A `BTreeMap`, so the keys
/// are read in order in every SDK.
fn decode_option_keys(chat_wire_shape: &str) -> BTreeMap<&'static str, OptionKey> {
    let mut keys = BTreeMap::new();
    for &provider in ALL_PROVIDER_NAMES {
        if with_wire_shape(provider_config(provider), chat_wire_shape).is_some() {
            keys.extend(provider_option_keys(provider, chat_wire_shape));
        }
    }
    keys
}

/// Maps every wire key the encoder may write for an option of (provider, wire
/// shape) to that option (RQC-008): the wire shape's key, which replaces the
/// others for every model (BUG-075), else the provider's key and each
/// per-model override key (ADR-024).
fn provider_option_keys(
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
/// `numbers` holds each number's text, as the body wrote it.
fn decode_options(
    request: &mut Request,
    config: &ProviderSpec,
    body: &mut Map<String, Value>,
    numbers: &NumberTexts,
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
    for (key, param) in decode_option_keys(config.chat_wire_shape) {
        let Some(value) = take_nested(container, key) else {
            continue;
        };
        if !seen.insert(param) {
            return Err(invalid(
                format!("{prefix}{key}"),
                format!("sets {param:?} a second time"),
            ));
        }
        let keys: Vec<String> = (!wrap.is_empty())
            .then_some(wrap)
            .into_iter()
            .chain(key.split('.'))
            .map(str::to_string)
            .collect();
        let text = numbers.get(&keys).map(String::as_str);
        set_option(request, param, &value, text)
            .map_err(|m| invalid(format!("{prefix}{key}"), m))?;
    }
    if !wrap.is_empty() {
        reject_unmodeled(container, &prefix)?;
    }
    Ok(())
}

/// Stores one decoded option value in its Request field, the inverse of
/// `add_options`. `text` is the value as written in the body. Returns the
/// problem with the value, if any.
fn set_option(
    request: &mut Request,
    param: OptionKey,
    value: &Value,
    text: Option<&str>,
) -> Result<(), &'static str> {
    let integer = || match (value, text) {
        (Value::Number(_), Some(text)) => integer_option(text),
        _ => Err("must be an integer"),
    };
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
        OptionKey::ThinkingType => {
            request.thinking_type = Some(value.as_str().ok_or("must be a string")?.to_string())
        }
        OptionKey::StopSequences => {
            // A bare string is a one-item list (HANDOFF-105 R3b).
            if let Value::String(stop) = value {
                request.stop = vec![stop.clone()];
                return Ok(());
            }
            request.stop = value
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .map(|s| s.as_str().map(str::to_string))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or("must be a string or a list of strings")?
        }
    }
    Ok(())
}

/// Reads an integer option from its JSON number text, exactly. JSON has one
/// number type, so an integral number in any notation (4096, 4096.0, 4.096e3)
/// is an integer, and a fraction is not (owner decision 3, HANDOFF-105 F3).
/// The text is read as digits times a power of ten, never as an f64, which
/// rounds past 2^53: 9007199254740993.0 is that integer, and
/// 9007199254740991.5 is a fraction (owner, 2026-11-01, HANDOFF-111). Go and
/// TS read the text the same way. A value outside i64 is out of range.
fn integer_option(text: &str) -> Result<i64, &'static str> {
    // serde_json checked the JSON grammar: -?int(.frac)?([eE][+-]?exp)?.
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fraction}");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Ok(0);
    }
    let significant = digits.trim_end_matches('0');
    // The value is significant * 10^scale. An exponent past i64 saturates;
    // the clamp keeps the sum below from overflowing, and any exponent near it
    // is out of range or a fraction all the same.
    let exponent = exponent
        .parse::<i64>()
        .unwrap_or(if exponent.starts_with('-') {
            i64::MIN
        } else {
            i64::MAX
        });
    let scale = exponent.clamp(-(1 << 40), 1 << 40) - fraction.len() as i64
        + (digits.len() - significant.len()) as i64;
    if scale < 0 {
        return Err("must be an integer");
    }
    if significant.len() as i64 + scale > 19 {
        return Err("is out of range");
    }
    // At most 19 digits, so below 10^19, which fits a u64.
    let magnitude = significant.parse::<u64>().map_err(|_| "is out of range")? as i128
        * 10i128.pow(scale as u32);
    let value = if negative { -magnitude } else { magnitude };
    i64::try_from(value).map_err(|_| "is out of range")
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
        // A wire role decodes through the provider's role mappings, then
        // through its decode-only ones (decodeOnly, HANDOFF-105 F3):
        // OpenAI and OpenRouter take developer as the system role. The role
        // mappings win, in every SDK; extract.py rejects a wire role that is
        // in both.
        let canonical = config
            .role_mappings
            .iter()
            .find(|(_, on_wire)| *on_wire == role)
            .map(|(canonical, _)| *canonical)
            .or_else(|| {
                config
                    .decode_role_mappings
                    .iter()
                    .find(|(on_wire, _)| *on_wire == role)
                    .map(|(_, canonical)| *canonical)
            });
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











































