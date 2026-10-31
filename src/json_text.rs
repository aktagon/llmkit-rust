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
//! the arguments, with no error. So `rust/clippy.toml` refuses serde_json's
//! writers and reqwest's `.json()` outside this file (HANDOFF-105 F8), and
//! the request-wire goldens catch the rest.

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
///
/// Text that is not one JSON value is held as it is, so `write_json` rejects
/// the text the caller gave. Compacting it first could make it JSON: "1 2"
/// would become "12" (HANDOFF-105 F9).
pub(crate) fn json_text(text: &str) -> Value {
    let held = match serde_json::from_str::<&RawValue>(text) {
        Ok(_) => compact_json(text),
        Err(_) => text.to_owned(),
    };
    let mut node = Map::new();
    node.insert(JSON_TEXT_KEY.to_owned(), Value::String(held));
    Value::Object(node)
}

/// `text`, valid JSON, with the whitespace between tokens removed and every
/// token kept as written (key order, number spelling, string escapes). JSON
/// whitespace is space, tab, line feed and carriage return (RFC 8259); any
/// other character stays, so a form feed is never removed.
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
        } else if !matches!(c, ' ' | '\t' | '\n' | '\r') {
            in_string = c == '"';
            out.push(c);
        }
    }
    out
}

/// `value` as compact JSON, as `serde_json::to_string` writes it, with each
/// [`json_text`] node written as its text. Text that is not one JSON value is
/// an error, so a written body is always JSON.
///
/// One difference: U+2028 and U+2029 are written as the escapes `\u2028` and
/// `\u2029`, as Go writes them, where serde_json writes the characters
/// (HANDOFF-105 F9, owner decision 1). A [`json_text`] node, the provider's or
/// the caller's text, is escaped so too (F10): the escape keeps the JSON value.
pub(crate) fn write_json(value: &Value) -> Result<String, serde_json::Error> {
    let mut out = String::new();
    write(value, &mut out)?;
    Ok(out)
}

// The one place serde_json writes: a key and a scalar, never a node
// (clippy.toml refuses serde_json's writers everywhere else).
#[allow(clippy::disallowed_methods)]
fn write(value: &Value, out: &mut String) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(map) => {
            if let Some(text) = spliced_text(map) {
                serde_json::from_str::<&RawValue>(text)?;
                push_escaped(out, text);
                return Ok(());
            }
            out.push('{');
            for (i, (key, member)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_escaped(out, &serde_json::to_string(key)?);
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
        scalar => push_escaped(out, &serde_json::to_string(scalar)?),
    }
    Ok(())
}

/// Pushes valid JSON text with U+2028 and U+2029 escaped. In valid JSON either
/// character can only stand inside a string, so its escape keeps the value.
fn push_escaped(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
}

fn spliced_text(map: &Map<String, Value>) -> Option<&str> {
    match (map.len(), map.get(JSON_TEXT_KEY)) {
        (1, Some(Value::String(text))) => Some(text),
        _ => None,
    }
}

// Tests may compare against serde_json; clippy.toml refuses it in library code.










































