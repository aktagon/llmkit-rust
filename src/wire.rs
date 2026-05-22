//! ADR-023 wire-format stability for serialized *Agent history.
//!
//! save_history + load_history are the ONLY guaranteed-stable
//! serialization path (STAB-009). Direct serde_json::to_string on a
//! Message will not compile today (ADR-020 KISS revert removed the
//! Serialize derive); when it does compile in a future ADR, the
//! bytes will still lack the `_v` envelope and load_history will
//! reject them with WireError::MissingVersion (STAB-011).
//!
//! The wire shape mirrors the canonical golden at
//! codegen/testdata/wire/v1/messages.json: a `_v` integer plus a
//! `messages` array of `{role, content, tool_calls, tool_result}`
//! objects.

use serde_json::{json, Map, Value};

use crate::structs::{Message, ToolCall, ToolResult};
use crate::wire_version::WIRE_SCHEMA_VERSION;

/// Wire-format error variants (ADR-023 STAB-003 + STAB-011).
#[derive(Debug)]
pub enum WireError {
    MissingVersion,
    UnsupportedVersion { got: i64, want: u32 },
    UnknownTopLevelKey(String),
    Malformed(String),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingVersion => write!(f, "llmkit: wire document missing _v key"),
            Self::UnsupportedVersion { got, want } => write!(
                f,
                "llmkit: unsupported wire schema version: got {got}, want <= {want}"
            ),
            Self::UnknownTopLevelKey(k) => {
                write!(f, "llmkit: unknown top-level wire key: {k:?}")
            }
            Self::Malformed(s) => write!(f, "llmkit: malformed wire document: {s}"),
        }
    }
}

impl std::error::Error for WireError {}

impl From<serde_json::Error> for WireError {
    fn from(err: serde_json::Error) -> Self {
        Self::Malformed(err.to_string())
    }
}

/// save_history serializes a slice of public Message values into the
/// canonical versioned wire document (ADR-023 STAB-002). Output
/// carries a `_v` integer and a `messages` array. tool_calls is
/// always an array (possibly empty); tool_result is always either an
/// object or JSON null (STAB-004 — never omitted).
pub fn save_history(messages: &[Message]) -> Result<String, WireError> {
    let wire: Vec<Value> = messages.iter().map(message_to_wire).collect();
    let doc = json!({
        "_v": WIRE_SCHEMA_VERSION,
        "messages": wire,
    });
    serde_json::to_string(&doc).map_err(WireError::from)
}

/// load_history parses a wire document and returns the in-memory
/// Message Vec. Returns WireError::MissingVersion when `_v` is
/// absent, WireError::UnsupportedVersion when `_v` exceeds the
/// SDK's WIRE_SCHEMA_VERSION, and WireError::UnknownTopLevelKey on a
/// top-level key outside (`_v`, `messages`, `_meta`). Unknown keys
/// nested inside Message/ToolCall/ToolResult are tolerated for
/// additive forward compatibility (STAB-003).
pub fn load_history(data: &str) -> Result<Vec<Message>, WireError> {
    let parsed: Value = serde_json::from_str(data)?;
    let obj = parsed
        .as_object()
        .ok_or_else(|| WireError::Malformed("wire document is not a JSON object".into()))?;
    let version = match obj.get("_v") {
        None => return Err(WireError::MissingVersion),
        Some(v) => v
            .as_i64()
            .ok_or_else(|| WireError::Malformed(format!("_v is not an integer: {v}")))?,
    };
    if version > i64::from(WIRE_SCHEMA_VERSION) {
        return Err(WireError::UnsupportedVersion {
            got: version,
            want: WIRE_SCHEMA_VERSION,
        });
    }
    for key in obj.keys() {
        if key != "_v" && key != "messages" && key != "_meta" {
            return Err(WireError::UnknownTopLevelKey(key.clone()));
        }
    }
    let raw_msgs = match obj.get("messages") {
        None => return Ok(Vec::new()),
        Some(v) => v
            .as_array()
            .ok_or_else(|| WireError::Malformed("messages is not an array".into()))?,
    };
    raw_msgs.iter().map(message_from_wire).collect()
}

fn message_to_wire(m: &Message) -> Value {
    let tool_calls: Vec<Value> = m.tool_calls.iter().map(tool_call_to_wire).collect();
    let tool_result = match &m.tool_result {
        Some(tr) => tool_result_to_wire(tr),
        None => Value::Null,
    };
    json!({
        "role": m.role,
        "content": m.content,
        "tool_calls": tool_calls,
        "tool_result": tool_result,
    })
}

fn tool_call_to_wire(tc: &ToolCall) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), Value::String(tc.id.clone()));
    out.insert("name".into(), Value::String(tc.name.clone()));
    // ADR-020 + STAB-004: input is Option<Value>; omit the key
    // entirely when None so providers that reject literal null
    // arguments stay happy on echo.
    if let Some(input) = &tc.input {
        out.insert("input".into(), input.clone());
    }
    Value::Object(out)
}

fn tool_result_to_wire(tr: &ToolResult) -> Value {
    json!({
        "tool_use_id": tr.tool_use_id,
        "content": tr.content,
    })
}

fn message_from_wire(raw: &Value) -> Result<Message, WireError> {
    let obj = raw
        .as_object()
        .ok_or_else(|| WireError::Malformed("message entry is not an object".into()))?;
    let role = obj
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let content = obj
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let tool_calls = match obj.get("tool_calls") {
        Some(Value::Array(arr)) => arr.iter().filter_map(tool_call_from_wire).collect(),
        _ => Vec::new(),
    };
    let tool_result = match obj.get("tool_result") {
        Some(Value::Object(tr)) => Some(ToolResult {
            tool_use_id: tr
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            content: tr
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        _ => None,
    };
    Ok(Message {
        role,
        content,
        tool_calls,
        tool_result,
    })
}

fn tool_call_from_wire(raw: &Value) -> Option<ToolCall> {
    let obj = raw.as_object()?;
    Some(ToolCall {
        id: obj
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        name: obj
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        input: obj.get("input").cloned(),
    })
}
