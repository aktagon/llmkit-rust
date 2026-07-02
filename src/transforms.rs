use std::collections::HashMap;

use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::providers::generated::providers::ProviderSpec;
use crate::providers::generated::request::{system_placement, tool_call_config, SystemPlacement};
use crate::structs::{Message, ToolCall, ToolResult};
use crate::types::Request;
use crate::Tool;

//
//
//
//
//
fn tool_call_input_value(call: &ToolCall) -> Value {
    match &call.input {
        Some(value) => value.clone(),
        None => Value::Object(Map::new()),
    }
}

pub(crate) fn apply_tool_defs(
    body: &mut Map<String, Value>,
    config: &ProviderSpec,
    tools: &[Tool],
) {
    if tools.is_empty() {
        return;
    }

    if config.chat_wire_shape == "ChatBedrock" {
        transform_bedrock_tool_defs(body, tools);
    } else if config.chat_wire_shape == "ChatGoogle" {
        //
        //
        //
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
    if config.chat_wire_shape == "ChatBedrock" {
        transform_bedrock_tool_call_msg(config, calls)
    } else if config.chat_wire_shape == "ChatGoogle" {
        transform_google_tool_call_msg(config, calls)
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        transform_anthropic_tool_call_msg(config, calls)
    } else {
        transform_openai_tool_call_msg(config, calls)
    }
}

pub(crate) fn tool_result_message(config: &ProviderSpec, result: &ToolResult) -> Value {
    if config.chat_wire_shape == "ChatBedrock" {
        transform_bedrock_tool_result_msg(result)
    } else if config.chat_wire_shape == "ChatGoogle" {
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
    if config.chat_wire_shape == "ChatBedrock" {
        extract_bedrock_tool_calls(raw)
    } else if config.chat_wire_shape == "ChatGoogle" {
        extract_google_tool_calls(raw)
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        extract_anthropic_tool_calls(raw)
    } else {
        extract_openai_tool_calls(raw, config)
    }
}

//
//
//

///
///
///
///
///
#[derive(Clone, Debug)]
pub(crate) enum Msg {
    ///
    Text { role: String, text: String },
    ///
    Calls(Vec<ToolCall>),
    ///
    Result(ToolResult),
}

///
///
///
///
///
///
///
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

//
//
//

///
///
///
pub(crate) fn apply_message_shape(
    body: &mut Map<String, Value>,
    msgs: &[Msg],
    request: &Request,
    config: &ProviderSpec,
) {
    if config.chat_wire_shape == "ChatGoogle" {
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
    let bedrock = config.chat_wire_shape == "ChatBedrock";
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
                    if bedrock {
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
        if bedrock {
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
        //
        //
        //
        //
        //
        //
        //
        //
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
    let is_anthropic = config.chat_wire_shape == "ChatAnthropic";
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


































































































































