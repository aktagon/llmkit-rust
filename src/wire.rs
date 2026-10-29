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
//!
//! integer plus a `messages` array of `{role, parts}` objects. Each part
//! is a JSON object with exactly one key, its variant name. Media bytes
//! are inline standard base64 (MSG-013).

use base64::Engine;
use serde_json::{json, Map, Value};

use crate::structs::{File, MediaRef, Message, Part, ProviderTurn, ToolCall, ToolResult};
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
                "llmkit: unsupported wire schema version: got {got}, want {want}"
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
/// canonical versioned wire document (ADR-023 STAB-002, version 2 per
/// ADR-100 MSG-012). Output carries a `_v` integer and a `messages`
/// array of `{role, parts}`. `parts` is always an array, empty when the
/// turn has none.
pub fn save_history(messages: &[Message]) -> Result<String, WireError> {
    let wire: Vec<Value> = messages.iter().map(message_to_wire).collect();
    let doc = json!({
        "_v": WIRE_SCHEMA_VERSION,
        "messages": wire,
    });
    serde_json::to_string(&doc).map_err(WireError::from)
}

/// load_history parses a wire document and returns the in-memory
/// Message Vec. Returns WireError::MissingVersion when `_v` is absent,
/// WireError::UnsupportedVersion when `_v` is not the SDK's
/// WIRE_SCHEMA_VERSION, and WireError::UnknownTopLevelKey on a top-level
/// key outside (`_v`, `messages`, `_meta`). A version 1 document is
/// rejected too: it has no migrator (ADR-100 MSG-014, an exception to
/// ADR-023 STAB-006), so the caller must rebuild it. A part with zero
/// keys, two keys or an unknown key is WireError::Malformed (MSG-012).
/// Unknown keys nested inside a Message or a part's payload are tolerated
/// for additive forward compatibility (STAB-003).
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
    if version != i64::from(WIRE_SCHEMA_VERSION) {
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
    let mut out = Map::new();
    out.insert("role".into(), Value::String(m.role.clone()));
    out.insert(
        "parts".into(),
        Value::Array(m.parts.iter().map(part_to_wire).collect()),
    );
    // ADR-085. Omitted when absent: a turn that never had a payload and a
    // turn whose payload was dropped are the same thing to a reader.
    // Omitting also keeps the canonical messages.json golden byte-identical,
    // since the wire.ttl fixture declares no payload.
    if let Some(turn) = &m.provider_turn {
        out.insert(
            "provider_turn".into(),
            json!({"wire_shape": turn.wire_shape, "wire": turn.wire}),
        );
    }
    Value::Object(out)
}

fn media_to_wire(m: &MediaRef) -> Value {
    json!({
        "mime_type": m.mime_type,
        "bytes": base64::engine::general_purpose::STANDARD.encode(&m.bytes),
    })
}

fn part_to_wire(p: &Part) -> Value {
    let value = match p {
        Part::Text(s) | Part::Lyrics(s) | Part::AudioUrl(s) => Value::String(s.clone()),
        Part::Image(m) | Part::Audio(m) => media_to_wire(m),
        Part::File(f) => json!({
            "id": f.id,
            "uri": f.uri,
            "mime_type": f.mime_type,
            "name": f.name,
        }),
        Part::ToolCall(tc) => {
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
        Part::ToolResult(tr) => json!({
            "tool_use_id": tr.tool_use_id,
            "content": tr.content,
        }),
    };
    let mut out = Map::new();
    out.insert(p.kind().into(), value);
    Value::Object(out)
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
    let parts = match obj.get("parts") {
        None => Vec::new(),
        Some(Value::Array(arr)) => arr.iter().map(part_from_wire).collect::<Result<_, _>>()?,
        Some(_) => return Err(WireError::Malformed("parts is not an array".into())),
    };
    // ADR-085 RSN-009, the unknown-field hazard. An OLD reader — one built
    // before provider_turn existed — silently drops this key, and on Anthropic a
    // dropped payload is a REJECTED request once the provider enforces the echo,
    // not a degraded one. Unknown-field tolerance is benign in general and is not
    // benign here. It is documented so a consumer reading a document written by
    // a newer SDK knows what a silent reconstruction means.
    let provider_turn = match obj.get("provider_turn") {
        Some(Value::Object(turn)) => Some(ProviderTurn {
            wire_shape: str_field(turn, "wire_shape"),
            wire: str_field(turn, "wire"),
        }),
        _ => None,
    };
    Ok(Message {
        role,
        parts,
        provider_turn,
    })
}

fn str_field(obj: &Map<String, Value>, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn object_of<'a>(key: &str, raw: &'a Value) -> Result<&'a Map<String, Value>, WireError> {
    raw.as_object()
        .ok_or_else(|| WireError::Malformed(format!("part {key:?} is not an object")))
}

fn string_of(key: &str, raw: &Value) -> Result<String, WireError> {
    raw.as_str()
        .map(str::to_string)
        .ok_or_else(|| WireError::Malformed(format!("part {key:?} is not a string")))
}

fn media_from_wire(key: &str, raw: &Value) -> Result<MediaRef, WireError> {
    let obj = object_of(key, raw)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(str_field(obj, "bytes"))
        .map_err(|err| WireError::Malformed(format!("part {key:?} bytes: {err}")))?;
    Ok(MediaRef {
        mime_type: str_field(obj, "mime_type"),
        bytes,
    })
}

fn part_from_wire(raw: &Value) -> Result<Part, WireError> {
    let obj = raw
        .as_object()
        .ok_or_else(|| WireError::Malformed("part is not an object".into()))?;
    if obj.len() != 1 {
        return Err(WireError::Malformed(format!(
            "a part has exactly one key, got {}",
            obj.len()
        )));
    }
    let (key, value) = obj.iter().next().expect("one key");
    Ok(match key.as_str() {
        "text" => Part::Text(string_of(key, value)?),
        "lyrics" => Part::Lyrics(string_of(key, value)?),
        "audio_url" => Part::AudioUrl(string_of(key, value)?),
        "image" => Part::Image(media_from_wire(key, value)?),
        "audio" => Part::Audio(media_from_wire(key, value)?),
        "file" => {
            let f = object_of(key, value)?;
            Part::File(File {
                id: str_field(f, "id"),
                uri: str_field(f, "uri"),
                mime_type: str_field(f, "mime_type"),
                name: str_field(f, "name"),
            })
        }
        "tool_call" => {
            let tc = object_of(key, value)?;
            Part::ToolCall(ToolCall {
                id: str_field(tc, "id"),
                name: str_field(tc, "name"),
                input: tc.get("input").cloned(),
            })
        }
        "tool_result" => {
            let tr = object_of(key, value)?;
            Part::ToolResult(ToolResult {
                tool_use_id: str_field(tr, "tool_use_id"),
                content: str_field(tr, "content"),
            })
        }
        other => return Err(WireError::Malformed(format!("unknown part key {other:?}"))),
    })
}
