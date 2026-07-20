//!
//!
//!
//!

use chrono::DateTime;
use serde_json::Value;

#
pub struct ParsedModelRecord {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub created: i64,
    pub context_window: i64,
    pub max_output: i64,
    pub raw: Option<Value>,
}

#
pub struct ParsedModelsPage {
    pub records: Vec<ParsedModelRecord>,
    pub next_cursor: String,
}

///
///
///
///
#
#
pub struct ParseError {
    pub provider: &'static str,
    pub reason: String,
}

fn parse_iso8601_best(s: &str) -> i64 {
    if s.is_empty() {
        return 0;
    }
    DateTime::parse_from_rfc3339(s).map(|dt| dt.timestamp()).unwrap_or(0)
}

fn s(v: &Value) -> String {
    v.as_str().map(|s| s.to_string()).unwrap_or_default()
}

fn n(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

//
//
static EMPTY: Vec<Value> = Vec::new();

///
pub fn parse_anthropic_models_response(body: &[u8]) -> Result<ParsedModelsPage, ParseError> {
    let envelope: Value = serde_json::from_slice(body).map_err(|e| ParseError {
        provider: "anthropic",
        reason: format!("envelope: {e}"),
    })?;
    let data = envelope.get("data").and_then(Value::as_array).unwrap_or(&EMPTY);
    let mut records = Vec::with_capacity(data.len());
    for wire in data {
        let max_out = match wire.get("max_output_tokens").and_then(Value::as_i64) {
            Some(v) if v > 0 => v,
            _ => wire.get("max_tokens").and_then(Value::as_i64).unwrap_or(0),
        };
        records.push(ParsedModelRecord {
            id: s(wire.get("id").unwrap_or(&Value::Null)),
            display_name: s(wire.get("display_name").unwrap_or(&Value::Null)),
            context_window: n(wire.get("max_input_tokens").unwrap_or(&Value::Null)),
            max_output: max_out,
            created: parse_iso8601_best(
                wire.get("created_at").and_then(Value::as_str).unwrap_or(""),
            ),
            description: String::new(),
            raw: Some(wire.clone()),
        });
    }
    let next_cursor = if envelope.get("has_more").and_then(Value::as_bool).unwrap_or(false) {
        envelope.get("last_id").and_then(Value::as_str).unwrap_or("").to_string()
    } else {
        String::new()
    };
    Ok(ParsedModelsPage { records, next_cursor })
}

///
///
///
///
pub fn parse_openai_cohort_models_response(body: &[u8]) -> Result<ParsedModelsPage, ParseError> {
    let parsed: Value = serde_json::from_slice(body).map_err(|e| ParseError {
        provider: "openai-cohort",
        reason: format!("envelope: {e}"),
    })?;
    let data = match parsed.as_array() {
        Some(arr) => arr,
        None => parsed.get("data").and_then(Value::as_array).unwrap_or(&EMPTY),
    };
    let records = data
        .iter()
        .map(|wire| ParsedModelRecord {
            id: s(wire.get("id").unwrap_or(&Value::Null)),
            created: n(wire.get("created").unwrap_or(&Value::Null)),
            raw: Some(wire.clone()),
            ..Default::default()
        })
        .collect();
    Ok(ParsedModelsPage { records, next_cursor: String::new() })
}

///
///
pub fn parse_google_models_response(body: &[u8]) -> Result<ParsedModelsPage, ParseError> {
    let envelope: Value = serde_json::from_slice(body).map_err(|e| ParseError {
        provider: "google",
        reason: format!("envelope: {e}"),
    })?;
    let data = envelope.get("models").and_then(Value::as_array).unwrap_or(&EMPTY);
    let prefix = "models/";
    let mut records = Vec::with_capacity(data.len());
    for wire in data {
        let mut id = s(wire.get("name").unwrap_or(&Value::Null));
        if let Some(stripped) = id.strip_prefix(prefix) {
            id = stripped.to_string();
        }
        records.push(ParsedModelRecord {
            id,
            display_name: s(wire.get("displayName").unwrap_or(&Value::Null)),
            description: s(wire.get("description").unwrap_or(&Value::Null)),
            context_window: n(wire.get("inputTokenLimit").unwrap_or(&Value::Null)),
            max_output: n(wire.get("outputTokenLimit").unwrap_or(&Value::Null)),
            created: 0,
            raw: Some(wire.clone()),
        });
    }
    let next_cursor = envelope.get("nextPageToken").and_then(Value::as_str).unwrap_or("").to_string();
    Ok(ParsedModelsPage { records, next_cursor })
}
