//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use serde_json::{json, Map, Value};

use crate::structs::{Message, ToolCall, ToolResult};
use crate::wire_version::WIRE_SCHEMA_VERSION;

///
#
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

///
///
///
///
///
pub fn save_history(messages: &[Message]) -> Result<String, WireError> {
    let wire: Vec<Value> = messages.iter().map(message_to_wire).collect();
    let doc = json!({
        "_v": WIRE_SCHEMA_VERSION,
        "messages": wire,
    });
    serde_json::to_string(&doc).map_err(WireError::from)
}

///
///
///
///
///
///
///
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
    //
    //
    //
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
