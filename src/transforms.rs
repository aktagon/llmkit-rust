use serde_json::{json, Map, Value};

use crate::providers::generated::providers::ProviderConfig;
use crate::providers::generated::request::{system_placement, tool_call_config, SystemPlacement};
use crate::Tool;

#[derive(Clone, Debug, Default)]
pub(crate) struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Map<String, Value>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
}

pub(crate) fn is_bedrock(config: &ProviderConfig) -> bool {
    config.wraps_options_in == "inferenceConfig" && config.auth_scheme == "SigV4"
}

pub(crate) fn apply_tool_defs(body: &mut Map<String, Value>, config: &ProviderConfig, tools: &[Tool]) {
    if tools.is_empty() {
        return;
    }

    if is_bedrock(config) {
        transform_bedrock_tool_defs(body, tools);
    } else if matches!(system_placement(config.name), SystemPlacement::SiblingObject) {
        transform_google_function_declarations(body, tools);
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        transform_anthropic_tools(body, tools);
    } else {
        transform_openai_functions(body, tools);
    }
}

pub(crate) fn tool_call_message(config: &ProviderConfig, calls: &[ToolCall]) -> Value {
    if is_bedrock(config) {
        transform_bedrock_tool_call_msg(config, calls)
    } else if matches!(system_placement(config.name), SystemPlacement::SiblingObject) {
        transform_google_tool_call_msg(config, calls)
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        transform_anthropic_tool_call_msg(config, calls)
    } else {
        transform_openai_tool_call_msg(config, calls)
    }
}

pub(crate) fn tool_result_message(config: &ProviderConfig, result: &ToolResult) -> Value {
    if is_bedrock(config) {
        transform_bedrock_tool_result_msg(result)
    } else if matches!(system_placement(config.name), SystemPlacement::SiblingObject) {
        transform_google_tool_result_msg(result)
    } else if tool_call_config(config.name)
        .is_some_and(|tool| tool.result_role == "user" && tool.args_format == "map")
    {
        transform_anthropic_tool_result_msg(result)
    } else {
        transform_openai_tool_result_msg(result)
    }
}

pub(crate) fn extract_tool_calls(raw: &Value, config: &ProviderConfig) -> Vec<ToolCall> {
    if is_bedrock(config) {
        extract_bedrock_tool_calls(raw)
    } else if matches!(system_placement(config.name), SystemPlacement::SiblingObject) {
        extract_google_tool_calls(raw)
    } else if tool_call_config(config.name).is_some_and(|tool| tool.args_format == "map") {
        extract_anthropic_tool_calls(raw)
    } else {
        extract_openai_tool_calls(raw, config)
    }
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

fn transform_google_function_declarations(body: &mut Map<String, Value>, tools: &[Tool]) {
    let decls: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.schema,
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

fn transform_openai_tool_call_msg(config: &ProviderConfig, calls: &[ToolCall]) -> Value {
    let tool_calls = calls
        .iter()
        .map(|call| {
            json!({
                "id": call.id,
                "type": "function",
                "function": {
                    "name": call.name,
                    "arguments": serde_json::to_string(&call.input).unwrap_or_else(|_| "{}".into()),
                }
            })
        })
        .collect::<Vec<_>>();

    json!({
        "role": map_role("assistant", config),
        "tool_calls": tool_calls,
    })
}

fn transform_anthropic_tool_call_msg(config: &ProviderConfig, calls: &[ToolCall]) -> Value {
    let content = calls
        .iter()
        .map(|call| {
            json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": call.input,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "role": map_role("assistant", config),
        "content": content,
    })
}

fn transform_google_tool_call_msg(config: &ProviderConfig, calls: &[ToolCall]) -> Value {
    let parts = calls
        .iter()
        .map(|call| {
            json!({
                "functionCall": {
                    "name": call.name,
                    "args": call.input,
                }
            })
        })
        .collect::<Vec<_>>();

    json!({
        "role": map_role("assistant", config),
        "parts": parts,
    })
}

fn transform_bedrock_tool_call_msg(config: &ProviderConfig, calls: &[ToolCall]) -> Value {
    let content = calls
        .iter()
        .map(|call| {
            json!({
                "toolUse": {
                    "toolUseId": call.id,
                    "name": call.name,
                    "input": call.input,
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

fn extract_openai_tool_calls(raw: &Value, config: &ProviderConfig) -> Vec<ToolCall> {
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
            let input = if args_format == "json_string" {
                function
                    .get("arguments")
                    .and_then(Value::as_str)
                    .and_then(|arguments| serde_json::from_str::<Map<String, Value>>(arguments).ok())
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
                input,
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
                input: block.get("input").and_then(Value::as_object).cloned().unwrap_or_default(),
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
                input: function_call.get("args").and_then(Value::as_object).cloned().unwrap_or_default(),
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
                input: tool_use.get("input").and_then(Value::as_object).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

fn map_role(role: &str, config: &ProviderConfig) -> String {
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
