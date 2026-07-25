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

/// Navigate a dotted path and return the value as an integer, or `None` when
/// the provider declares no path for this field or the response did not carry
/// it (ADR-081). The distinction a plain zero-defaulting reader cannot draw: a
/// provider reporting `cached_tokens: 0` and a provider that never mentions
/// caching are different claims, and neither of them is the number zero.
pub fn opt_int_path(data: &Value, path: &str) -> Option<i64> {
    if path.is_empty() {
        return None;
    }
    match navigate_path(data, path) {
        Some(Value::Number(value)) => value.as_i64(),
        _ => None,
    }
}

/// [`opt_int_path`] for the fractional provider-reported USD cost (ADR-027).
/// An unreported cost is not a free request (AVAIL-007).
pub fn opt_f64_path(data: &Value, path: &str) -> Option<f64> {
    if path.is_empty() {
        return None;
    }
    match navigate_path(data, path) {
        Some(Value::Number(value)) => value.as_f64(),
        _ => None,
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
/// An empty path (the provider declares no location for this field) or a `Null`
/// value (the canonical field was never reported) is a no-op: there is nothing
/// to write, and materializing a field the provider never sent would invent
/// one. A reported zero IS written — see `is_empty_wire_value`.
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

/// Whether `value` carries nothing to write. `Null` is the caller saying the
/// field was never reported, so there is no location to fill.
///
/// A numeric zero is NOT empty (ADR-081). It used to be: the encoder dropped
/// every zero, so a body that explicitly said `cached_tokens: 0` round-tripped
/// to one that omitted the field — the reader then had to guess, and guessed
/// zero, which happened to look right. Now absence arrives as `Null` and a
/// reported zero arrives as `0`, so the encoder can tell them apart instead of
/// inferring one from the other.
fn is_empty_wire_value(value: &Value) -> bool {
    match value {
        Value::String(text) => text.is_empty(),
        Value::Null => true,
        _ => false,
    }
}
