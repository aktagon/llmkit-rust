use serde_json::Value;

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
