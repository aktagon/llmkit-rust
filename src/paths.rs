use serde_json::{Map, Value};

pub fn extract_string_path(data: &Value, path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    match navigate_path(data, path) {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub fn extract_u32_path(data: &Value, path: &str) -> u32 {
    match navigate_path(data, path) {
        Some(Value::Number(value)) => value.as_u64().unwrap_or_default() as u32,
        _ => 0,
    }
}

/// Navigate a dotted path and return the value as f64, or 0.0 on miss.
/// Used for provider-reported USD cost (ADR-027), which is fractional.
pub fn extract_f64_path(data: &Value, path: &str) -> f64 {
    if path.is_empty() {
        return 0.0;
    }
    match navigate_path(data, path) {
        Some(Value::Number(value)) => value.as_f64().unwrap_or_default(),
        _ => 0.0,
    }
}

fn navigate_path<'a>(data: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = data;
    for part in path.split('.') {
        if let Some(index_start) = part.find('[') {
            let field = &part[..index_start];
            let index_end = part.find(']')?;
            let index: usize = part[index_start + 1..index_end].parse().ok()?;
            current = current.get(field)?;
            current = current.get(index)?;
        } else {
            current = current.get(part)?;
        }
    }
    Some(current)
}

/// Place `value` at a dot-notation path with array index support
/// (choices[0].message.content), creating intermediate objects and array
/// elements as it descends. It is the navigate-or-create inverse of
/// `navigate_path` and walks the identical generated path strings (ADR-076
/// SYM-005). ANY segment may be indexed, not just the first: Google's response
/// text path is candidates[0].content.parts[0].text — two array levels created
/// in a single descent.
///
/// An empty path (the provider declares no location for this field) or an empty
/// value is a no-op: there is nothing to write, and materializing a zero would
/// invent a field the provider never sent.
pub fn set_wire_path(data: &mut Value, path: &str, value: Value) {
    if path.is_empty() || is_empty_wire_value(&value) {
        return;
    }
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = data;
    for (i, part) in parts.iter().copied().enumerate() {
        let (field, index) = split_index(part);
        current = member(current, field);
        if let Some(index) = index {
            current = element(current, index);
        }
        if i == parts.len() - 1 {
            *current = value;
            return;
        }
    }
}

/// Split "parts[0]" into ("parts", Some(0)) and "content" into ("content",
/// None), mirroring `navigate_path`'s bracket handling.
fn split_index(part: &str) -> (&str, Option<usize>) {
    match (part.find('['), part.find(']')) {
        (Some(start), Some(end)) if start < end => {
            (&part[..start], part[start + 1..end].parse().ok())
        }
        _ => (part, None),
    }
}

/// The slot at `value[field]`, coercing `value` to an object and creating the
/// member when absent — the create half of `navigate_path`'s `get(field)`.
fn member<'a>(value: &'a mut Value, field: &str) -> &'a mut Value {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    match value {
        Value::Object(map) => map.entry(field).or_insert(Value::Null),
        // Coerced to an object on the line above; the arm stays total rather
        // than panicking on a state the branch cannot be in.
        other => other,
    }
}

/// The slot at `value[index]`, coercing `value` to an array and padding it with
/// nulls up to `index`.
fn element(value: &mut Value, index: usize) -> &mut Value {
    if !value.is_array() {
        *value = Value::Array(Vec::new());
    }
    match value {
        Value::Array(items) => {
            while items.len() <= index {
                items.push(Value::Null);
            }
            &mut items[index]
        }
        other => other,
    }
}

/// Whether `value` is the zero of its canonical type. Empty values are skipped
/// rather than written, so the encoder never claims a provider reported zero
/// tokens when the canonical `Response` simply had none.
fn is_empty_wire_value(value: &Value) -> bool {
    match value {
        Value::String(text) => text.is_empty(),
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::Null => true,
        _ => false,
    }
}
