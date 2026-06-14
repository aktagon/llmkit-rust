use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::providers::generated::providers::ProviderSpec;
use crate::providers::generated::request::{system_placement, tool_call_config, SystemPlacement};
use crate::structs::{Message, ToolCall, ToolResult};
use crate::types::Request;
use crate::Tool;

// ADR-020 promoted ToolCall + ToolResult into public crate::structs. The
// generated `ToolCall.input` is `Option<serde_json::Value>` (was a
// private `Map<String, Value>` here). `tool_call_input_value` returns
// the input as a JSON value, defaulting to `{}` when None or
// non-object so providers that reject literal null inputs stay happy.
fn tool_call_input_value(call: &ToolCall) -> Value {
    match &call.input {
        Some(value) => value.clone(),
        None => Value::Object(Map::new()),
    }
}

pub(crate) fn is_bedrock(config: &ProviderSpec) -> bool {
    config.wraps_options_in == "inferenceConfig" && config.auth_scheme == "SigV4"
}

pub(crate) fn apply_tool_defs(
    body: &mut Map<String, Value>,
    config: &ProviderSpec,
    tools: &[Tool],
) {
    if tools.is_empty() {
        return;
    }

    if is_bedrock(config) {
        transform_bedrock_tool_defs(body, tools);
    } else if matches!(
        system_placement(config.name),
        SystemPlacement::SiblingObject
    ) {
        // Google carries tool params under a per-provider wire field (ADR-025):
        // "parametersJsonSchema" accepts native JSON Schema verbatim, vs the
        // OpenAPI-3.0-subset "parameters" default.
        let field = tool_call_config(config.name)
            .map(|tc| tc.params_wire_field)
            .filter(|f| !f.is_empty())
            .unwrap_or("parameters");
        transform_google_function_declarations(body, tools, field);
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        transform_anthropic_tools(body, tools);
    } else {
        transform_openai_functions(body, tools);
    }
}

pub(crate) fn tool_call_message(config: &ProviderSpec, calls: &[ToolCall]) -> Value {
    if is_bedrock(config) {
        transform_bedrock_tool_call_msg(config, calls)
    } else if matches!(
        system_placement(config.name),
        SystemPlacement::SiblingObject
    ) {
        transform_google_tool_call_msg(config, calls)
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        transform_anthropic_tool_call_msg(config, calls)
    } else {
        transform_openai_tool_call_msg(config, calls)
    }
}

pub(crate) fn tool_result_message(config: &ProviderSpec, result: &ToolResult) -> Value {
    if is_bedrock(config) {
        transform_bedrock_tool_result_msg(result)
    } else if matches!(
        system_placement(config.name),
        SystemPlacement::SiblingObject
    ) {
        transform_google_tool_result_msg(result)
    } else if tool_call_config(config.name)
        .is_some_and(|tool| tool.result_role == "user" && tool.args_format == "map")
    {
        transform_anthropic_tool_result_msg(result)
    } else {
        transform_openai_tool_result_msg(result)
    }
}

pub(crate) fn extract_tool_calls(raw: &Value, config: &ProviderSpec) -> Vec<ToolCall> {
    if is_bedrock(config) {
        extract_bedrock_tool_calls(raw)
    } else if matches!(
        system_placement(config.name),
        SystemPlacement::SiblingObject
    ) {
        extract_google_tool_calls(raw)
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        extract_anthropic_tool_calls(raw)
    } else {
        extract_openai_tool_calls(raw, config)
    }
}

// =============================================================================
// Internal message sum (ADR-026 PIPE-007/008)
// =============================================================================

/// The internal message representation: a sum that is *exactly one of* text,
/// tool-calls, or tool-result. The public [`Message`] (structs.rs) is a flat
/// product that can encode an illegal multi-carrier combination; this enum
/// cannot, so the transforms below dispatch with an exhaustive `match` and the
/// compiler — not a runtime guard — rejects any unhandled variant.
#[derive(Clone, Debug)]
pub(crate) enum Msg {
    /// A plain conversational turn: a role and its text content, nothing else.
    Text { role: String, text: String },
    /// An assistant turn that issued one or more tool invocations.
    Calls(Vec<ToolCall>),
    /// A tool turn carrying exactly one execution result.
    Result(ToolResult),
}

/// Converts the public, untrusted [`Message`] slice into the internal sum.
/// This is the single carrier-validation boundary (PIPE-008): a message
/// carrying more than one of {content, tool calls, tool result} is rejected
/// here, not silently mis-serialized downstream. The Text/batch/stream paths
/// feed user-supplied Message lists through here; the Agent builds the sum
/// directly from its trusted history (`Agent::history_to_msgs`) and so skips
/// this check.
pub(crate) fn to_internal(messages: &[Message]) -> Result<Vec<Msg>, Error> {
    let mut out = Vec::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        let carriers = u8::from(m.tool_result.is_some())
            + u8::from(!m.tool_calls.is_empty())
            + u8::from(!m.content.is_empty());
        if carriers > 1 {
            return Err(Error::Validation {
                field: "messages",
                message: format!(
                    "messages[{i}] must carry only one of content, tool calls, or tool result"
                ),
            });
        }
        if let Some(result) = &m.tool_result {
            out.push(Msg::Result(result.clone()));
        } else if !m.tool_calls.is_empty() {
            out.push(Msg::Calls(m.tool_calls.clone()));
        } else {
            out.push(Msg::Text {
                role: m.role.clone(),
                text: m.content.clone(),
            });
        }
    }
    Ok(out)
}

// =============================================================================
// Message transforms — build the messages/contents array in request body
// =============================================================================

/// Builds the provider-specific messages/contents array. Selected by
/// [`ProviderSpec`] fields (not provider name), mirroring the tool transform
/// selectors above.
pub(crate) fn apply_message_shape(
    body: &mut Map<String, Value>,
    msgs: &[Msg],
    request: &Request,
    config: &ProviderSpec,
) {
    if matches!(
        system_placement(config.name),
        SystemPlacement::SiblingObject
    ) {
        transform_google_parts(body, msgs, request, config);
    } else {
        transform_flat_content(body, msgs, request, config);
    }
}

fn transform_flat_content(
    body: &mut Map<String, Value>,
    msgs: &[Msg],
    request: &Request,
    config: &ProviderSpec,
) {
    let is_bedrock = is_bedrock(config);
    let mut messages = Vec::new();

    if matches!(
        system_placement(config.name),
        SystemPlacement::MessageInArray
    ) {
        if let Some(system) = &request.system {
            messages.push(json!({
                "role": map_role("system", config),
                "content": system,
            }));
        }
    }

    if !msgs.is_empty() {
        for m in msgs {
            match m {
                Msg::Result(result) => messages.push(tool_result_message(config, result)),
                Msg::Calls(calls) => messages.push(tool_call_message(config, calls)),
                Msg::Text { role, text } => {
                    if is_bedrock {
                        messages.push(json!({
                            "role": map_role(role, config),
                            "content": [{"text": text}],
                        }));
                    } else {
                        messages.push(json!({
                            "role": map_role(role, config),
                            "content": text,
                        }));
                    }
                }
            }
        }
    } else if let Some(user) = &request.user {
        if is_bedrock {
            messages.push(json!({
                "role": map_role("user", config),
                "content": [{"text": user}],
            }));
        } else if !request.files.is_empty() || !request.images.is_empty() {
            messages.push(json!({
                "role": map_role("user", config),
                "content": build_flat_content_parts(request, config),
            }));
        } else {
            messages.push(json!({
                "role": map_role("user", config),
                "content": user,
            }));
        }
    }

    body.insert("messages".into(), Value::Array(messages));
}

fn transform_google_parts(
    body: &mut Map<String, Value>,
    msgs: &[Msg],
    request: &Request,
    config: &ProviderSpec,
) {
    let mut contents = Vec::new();

    if !msgs.is_empty() {
        // Google's wire identifies a tool result by the function NAME, but the
        // universal ToolResult carries only tool_use_id. Recover id->name from
        // the call turns, which always precede their result in a valid history,
        // and resolve the result's name from it. A NEW ToolResult is built (not
        // mutated) so the caller's Message history is untouched — the slice is
        // borrowed, and the inner ToolResult is shared. The agent path is
        // unaffected (its extractor sets id==name); an unmatched id passes
        // through unchanged.
        let mut id_to_name: HashMap<String, String> = HashMap::new();
        for m in msgs {
            match m {
                Msg::Result(result) => {
                    let resolved = match id_to_name.get(&result.tool_use_id) {
                        Some(name) => ToolResult {
                            tool_use_id: name.clone(),
                            content: result.content.clone(),
                        },
                        None => result.clone(),
                    };
                    contents.push(tool_result_message(config, &resolved));
                }
                Msg::Calls(calls) => {
                    for call in calls {
                        id_to_name.insert(call.id.clone(), call.name.clone());
                    }
                    contents.push(tool_call_message(config, calls));
                }
                Msg::Text { role, text } => {
                    contents.push(json!({
                        "role": map_role(role, config),
                        "parts": [{"text": text}],
                    }));
                }
            }
        }
    } else if let Some(user) = &request.user {
        let parts = build_google_parts(request).unwrap_or_else(|| vec![json!({"text": user})]);
        contents.push(json!({
            "role": map_role("user", config),
            "parts": parts,
        }));
    }

    body.insert("contents".into(), Value::Array(contents));
}

fn build_flat_content_parts(request: &Request, config: &ProviderSpec) -> Vec<Value> {
    let is_anthropic = matches!(
        system_placement(config.name),
        SystemPlacement::TopLevelField
    );
    let mut parts = Vec::new();

    for file in &request.files {
        if is_anthropic {
            parts.push(json!({
                "type": "document",
                "source": {"type": "file", "file_id": file.id},
            }));
        } else {
            parts.push(json!({
                "type": "file",
                "file": {"file_id": file.id},
            }));
        }
    }

    for image in &request.images {
        if is_anthropic {
            if image.url.starts_with("data:") {
                let (mime_type, data) = parse_data_uri(&image.url);
                parts.push(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": mime_type, "data": data},
                }));
            } else {
                parts.push(json!({
                    "type": "image",
                    "source": {"type": "url", "url": image.url},
                }));
            }
        } else {
            let detail = if image.detail.is_empty() {
                "auto"
            } else {
                &image.detail
            };
            parts.push(json!({
                "type": "image_url",
                "image_url": {"url": image.url, "detail": detail},
            }));
        }
    }

    if let Some(user) = &request.user {
        parts.push(json!({"type": "text", "text": user}));
    }
    parts
}

fn build_google_parts(request: &Request) -> Option<Vec<Value>> {
    if request.files.is_empty() && request.images.is_empty() {
        return request
            .user
            .as_ref()
            .map(|user| vec![json!({"text": user})]);
    }

    let mut parts = Vec::new();
    for file in &request.files {
        parts.push(json!({
            "file_data": {"file_uri": file.uri, "mime_type": file.mime_type}
        }));
    }
    for image in &request.images {
        if image.url.starts_with("data:") {
            let (mime_type, data) = parse_data_uri(&image.url);
            parts.push(json!({
                "inline_data": {"mime_type": mime_type, "data": data}
            }));
        }
    }
    if let Some(user) = &request.user {
        parts.push(json!({"text": user}));
    }
    Some(parts)
}

fn parse_data_uri(uri: &str) -> (String, String) {
    if !uri.starts_with("data:") {
        return (String::new(), uri.to_string());
    }
    let remainder = &uri["data:".len()..];
    let mut parts = remainder.splitn(2, ',');
    let meta = parts.next().unwrap_or_default();
    let data = parts.next().unwrap_or_default();
    (
        meta.trim_end_matches(";base64").to_string(),
        data.to_string(),
    )
}

fn transform_openai_functions(body: &mut Map<String, Value>, tools: &[Tool]) {
    let defs = tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.schema,
                }
            })
        })
        .collect();
    body.insert("tools".into(), Value::Array(defs));
}

fn transform_anthropic_tools(body: &mut Map<String, Value>, tools: &[Tool]) {
    let defs = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.schema,
            })
        })
        .collect();
    body.insert("tools".into(), Value::Array(defs));
}

fn transform_google_function_declarations(
    body: &mut Map<String, Value>,
    tools: &[Tool],
    params_wire_field: &str,
) {
    let decls: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                params_wire_field: tool.schema,
            })
        })
        .collect();
    body.insert("tools".into(), json!([{ "functionDeclarations": decls }]));
}

fn transform_bedrock_tool_defs(body: &mut Map<String, Value>, tools: &[Tool]) {
    let defs: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "toolSpec": {
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": {"json": tool.schema},
                }
            })
        })
        .collect();
    body.insert("toolConfig".into(), json!({"tools": defs}));
}

fn transform_openai_tool_call_msg(config: &ProviderSpec, calls: &[ToolCall]) -> Value {
    let tool_calls = calls
        .iter()
        .map(|call| {
            json!({
                "id": call.id,
                "type": "function",
                "function": {
                    "name": call.name,
                    "arguments": serde_json::to_string(&tool_call_input_value(call)).unwrap_or_else(|_| "{}".into()),
                }
            })
        })
        .collect::<Vec<_>>();

    json!({
        "role": map_role("assistant", config),
        "tool_calls": tool_calls,
    })
}

fn transform_anthropic_tool_call_msg(config: &ProviderSpec, calls: &[ToolCall]) -> Value {
    let content = calls
        .iter()
        .map(|call| {
            json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": tool_call_input_value(call),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "role": map_role("assistant", config),
        "content": content,
    })
}

fn transform_google_tool_call_msg(config: &ProviderSpec, calls: &[ToolCall]) -> Value {
    let parts = calls
        .iter()
        .map(|call| {
            json!({
                "functionCall": {
                    "name": call.name,
                    "args": tool_call_input_value(call),
                }
            })
        })
        .collect::<Vec<_>>();

    json!({
        "role": map_role("assistant", config),
        "parts": parts,
    })
}

fn transform_bedrock_tool_call_msg(config: &ProviderSpec, calls: &[ToolCall]) -> Value {
    let content = calls
        .iter()
        .map(|call| {
            json!({
                "toolUse": {
                    "toolUseId": call.id,
                    "name": call.name,
                    "input": tool_call_input_value(call),
                }
            })
        })
        .collect::<Vec<_>>();

    json!({
        "role": map_role("assistant", config),
        "content": content,
    })
}

fn transform_openai_tool_result_msg(result: &ToolResult) -> Value {
    json!({
        "role": "tool",
        "content": result.content,
        "tool_call_id": result.tool_use_id,
    })
}

fn transform_anthropic_tool_result_msg(result: &ToolResult) -> Value {
    json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": result.tool_use_id,
            "content": result.content,
        }],
    })
}

fn transform_google_tool_result_msg(result: &ToolResult) -> Value {
    json!({
        "role": "user",
        "parts": [{
            "functionResponse": {
                "name": result.tool_use_id,
                "response": {"result": result.content},
            }
        }],
    })
}

fn transform_bedrock_tool_result_msg(result: &ToolResult) -> Value {
    json!({
        "role": "user",
        "content": [{
            "toolResult": {
                "toolUseId": result.tool_use_id,
                "content": [{"text": result.content}],
            }
        }],
    })
}

fn extract_openai_tool_calls(raw: &Value, config: &ProviderSpec) -> Vec<ToolCall> {
    let Some(calls) = raw
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("tool_calls"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let args_format = tool_call_config(config.name)
        .map(|tool| tool.args_format)
        .unwrap_or("json_string");

    calls
        .iter()
        .filter_map(|call| {
            let function = call.get("function")?;
            let name = function.get("name")?.as_str()?.to_string();
            let input_map: Map<String, Value> = if args_format == "json_string" {
                function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|arguments| {
                        serde_json::from_str::<Map<String, Value>>(arguments).ok()
                    })
                    .unwrap_or_default()
            } else {
                function
                    .get("arguments")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default()
            };

            Some(ToolCall {
                id: stringify(call.get("id")),
                name,
                input: Some(Value::Object(input_map)),
            })
        })
        .collect()
}

fn extract_anthropic_tool_calls(raw: &Value) -> Vec<ToolCall> {
    raw.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                return None;
            }
            Some(ToolCall {
                id: stringify(block.get("id")),
                name: stringify(block.get("name")),
                input: Some(Value::Object(
                    block
                        .get("input")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                )),
            })
        })
        .collect()
}

fn extract_google_tool_calls(raw: &Value) -> Vec<ToolCall> {
    raw.get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| {
            let function_call = part.get("functionCall")?;
            let name = stringify(function_call.get("name"));
            Some(ToolCall {
                id: name.clone(),
                name,
                input: Some(Value::Object(
                    function_call
                        .get("args")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                )),
            })
        })
        .collect()
}

fn extract_bedrock_tool_calls(raw: &Value) -> Vec<ToolCall> {
    raw.get("output")
        .and_then(|output| output.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|block| {
            let tool_use = block.get("toolUse")?;
            Some(ToolCall {
                id: stringify(tool_use.get("toolUseId")),
                name: stringify(tool_use.get("name")),
                input: Some(Value::Object(
                    tool_use
                        .get("input")
                        .and_then(Value::as_object)
                        .cloned()
                        .unwrap_or_default(),
                )),
            })
        })
        .collect()
}

fn map_role(role: &str, config: &ProviderSpec) -> String {
    config
        .role_mappings
        .iter()
        .find(|(from, _)| *from == role)
        .map(|(_, to)| (*to).to_string())
        .unwrap_or_else(|| role.to_string())
}

fn stringify(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(string)) => string.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(boolean)) => boolean.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::PromptOptions;
    use crate::providers::generated::providers::{provider_config, ProviderName};
    use crate::types::{Provider, Request};

    fn snapshot_options() -> PromptOptions {
        let mut o = PromptOptions::new();
        o.max_tokens = Some(256);
        o.temperature = Some(0.1);
        o.top_p = Some(0.5);
        o
    }

    // ADR-026 regression net (Rust slice). build_request is the single body
    // builder shared by Text, batch, stream, and — after this slice — the
    // Agent. These snapshots freeze the Text wire body per provider shape and
    // MUST stay byte-equal now the Agent routes through build_request
    // (PIPE-005). serde_json sorts object keys (BTreeMap), so the comparison is
    // deterministic and matches the Go/TS/Python slices' snapshots.
    #[test]
    fn build_request_wire_body_snapshots() {
        let req = Request {
            system: Some("be terse".into()),
            user: Some("hello".into()),
            ..Request::default()
        };
        let opts = snapshot_options();
        let cases: &[(ProviderName, &str)] = &[
            (
                ProviderName::Anthropic,
                r#"{"max_tokens":256,"messages":[{"content":"hello","role":"user"}],"model":"claude-sonnet-4-6","system":"be terse","temperature":0.1,"top_p":0.5}"#,
            ),
            (
                ProviderName::OpenAI,
                r#"{"max_tokens":256,"messages":[{"content":"be terse","role":"system"},{"content":"hello","role":"user"}],"model":"gpt-4o-2024-08-06","temperature":0.1,"top_p":0.5}"#,
            ),
            (
                ProviderName::Google,
                r#"{"contents":[{"parts":[{"text":"hello"}],"role":"user"}],"generationConfig":{"max_output_tokens":256,"temperature":0.1,"top_p":0.5},"system_instruction":{"parts":[{"text":"be terse"}]}}"#,
            ),
            (
                ProviderName::Bedrock,
                r#"{"inferenceConfig":{"maxTokens":256,"temperature":0.1,"top_p":0.5},"messages":[{"content":[{"text":"hello"}],"role":"user"}],"system":[{"text":"be terse"}]}"#,
            ),
        ];
        for (name, want) in cases {
            let provider = Provider::new(*name, "k");
            let msgs = to_internal(&req.messages).expect("to_internal");
            let (body, _) = crate::request::build_request(&provider, &req, &msgs, &opts, &[])
                .expect("build_request");
            let got = serde_json::to_string(&body).expect("serialize body");
            assert_eq!(&got.as_str(), want, "wire body drift for {name:?}");
        }
    }

    // Locks the ADR-026 #2 fix. Google's wire identifies a tool result by
    // function NAME, but the universal ToolResult carries only tool_use_id. On
    // the Text/batch path a user supplies a history where the id differs from
    // the name (unlike the agent, whose extractor sets id==name), so the
    // result's functionResponse.name must be resolved back to the function name
    // via the preceding tool-call turn — not echo the raw id.
    #[test]
    fn google_tool_result_resolves_function_name() {
        let req = Request {
            messages: vec![
                Message::new("user", "weather in Paris?"),
                Message {
                    role: "assistant".into(),
                    tool_calls: vec![ToolCall {
                        id: "call_abc123".into(),
                        name: "get_weather".into(),
                        input: Some(json!({"city": "Paris"})),
                    }],
                    ..Default::default()
                },
                Message {
                    role: "tool".into(),
                    tool_result: Some(ToolResult {
                        tool_use_id: "call_abc123".into(),
                        content: "sunny, 21C".into(),
                    }),
                    ..Default::default()
                },
            ],
            ..Request::default()
        };
        let cfg = provider_config(ProviderName::Google);
        let msgs = to_internal(&req.messages).expect("to_internal");
        let mut body = Map::new();
        apply_message_shape(&mut body, &msgs, &req, cfg);

        let contents = body["contents"].as_array().expect("contents array");
        assert_eq!(contents.len(), 3, "expected 3 contents");
        let parts = contents[2]["parts"].as_array().expect("parts array");
        let name = parts[0]["functionResponse"]["name"]
            .as_str()
            .expect("functionResponse.name");
        assert_eq!(name, "get_weather", "name resolved from preceding call id");
    }

    // PIPE-008: the carrier-validation boundary rejects a public Message that
    // sets more than one of {content, tool calls, tool result}.
    #[test]
    fn to_internal_rejects_multi_carrier_message() {
        let messages = vec![Message {
            role: "assistant".into(),
            content: "text and a tool call".into(),
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "noop".into(),
                input: None,
            }],
            ..Default::default()
        }];
        let err = to_internal(&messages).expect_err("multi-carrier must be rejected");
        assert!(
            matches!(
                err,
                Error::Validation {
                    field: "messages",
                    ..
                }
            ),
            "expected a messages validation error, got {err:?}"
        );
    }
}
