//! The one JSON writer for every body llmkit sends or encodes: the chat
//! request on every send path, `encode_request`, `encode_response` and
//! `save_history` (HANDOFF-105 F7, owner decision 1).
//!
//! A tool call's arguments are JSON text (`ToolCall.input`). A map-format
//! argument (Anthropic, Google, Bedrock) is a value inside the body, and the
//! body is a `serde_json::Value`, which sorts its keys. So the body holds the
//! text in a node built by [`json_text`], and [`write_json`] writes that text
//! where the value goes. This is the splice, with no serde_json feature: a
//! body serialized any other way carries the node's reserved key instead of
//! the arguments, which the request-wire goldens catch.

use serde_json::value::RawValue;
use serde_json::{Map, Value};

/// The one key of a node that holds JSON text. A NUL byte starts it, so no
/// provider field and no caller key is this string by accident.
const JSON_TEXT_KEY: &str = "\u{0}llmkit:json_text";

/// A body node that [`write_json`] writes as `text`, which must be one JSON
/// value; `write_json` rejects it otherwise. The node holds the text compacted,
/// once: the tokens (key order, number spelling, escapes) stay, the whitespace
/// between them goes, as Go's encoder writes an embedded value (HANDOFF-105
/// F8, owner decision 1).
pub(crate) fn json_text(text: &str) -> Value {
    let mut node = Map::new();
    node.insert(JSON_TEXT_KEY.to_owned(), Value::String(compact_json(text)));
    Value::Object(node)
}

/// `text`, valid JSON, with the whitespace between tokens removed and every
/// token kept as written (key order, number spelling, string escapes).
pub(crate) fn compact_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_string, mut escaped) = (false, false);
    for c in text.chars() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if !c.is_ascii_whitespace() {
            in_string = c == '"';
            out.push(c);
        }
    }
    out
}

/// `value` as compact JSON, as `serde_json::to_string` writes it, with each
/// [`json_text`] node written as its text. Text that is not one JSON value is
/// an error, so a written body is always JSON.
pub(crate) fn write_json(value: &Value) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    write(value, &mut out)?;
    Ok(out)
}

fn write(value: &Value, out: &mut String) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(map) => {
            if let Some(text) = spliced_text(map) {
                serde_json::from_str::<&RawValue>(text)?;
                out.push_str(text);
                return Ok(());
            }
            out.push('{');
            for (i, (key, member)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key)?);
                out.push(':');
                write(member, out)?;
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write(item, out)?;
            }
            out.push(']');
        }
        scalar => out.push_str(&serde_json::to_string(scalar)?),
    }
    Ok(())
}

fn spliced_text(map: &Map<String, Value>) -> Option<&str> {
    match (map.len(), map.get(JSON_TEXT_KEY)) {
        (1, Some(Value::String(text))) => Some(text),
        _ => None,
    }
}






























