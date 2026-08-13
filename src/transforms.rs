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
    } else if config.chat_wire_shape == "ChatResponsesOpenAI" {
        transform_responses_tool_result_msg(result)
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
    /// An assistant turn the provider itself serialized, replayed verbatim
    /// instead of rebuilt (ADR-085). It carries the projection it replaces so
    /// [`crate::provider_turn::resolve_turns`] can drop back to reconstruction
    /// when the payload was captured under a different wire shape — the
    /// alternative, deciding that at transform time, would put the same check
    /// in three places.
    Turn {
        shape: String,
        wire: String,
        fallback: Box<Msg>,
    },
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
        let projected = if let Some(result) = &m.tool_result {
            Msg::Result(result.clone())
        } else if !m.tool_calls.is_empty() {
            Msg::Calls(m.tool_calls.clone())
        } else {
            Msg::Text {
                role: m.role.clone(),
                text: m.content.clone(),
            }
        };
        // provider_turn is not a fourth carrier — it is the same turn in the
        // provider's own serialization, so it never participates in the
        // one-carrier check above. When present it supersedes the projection on
        // the wire while the projection stays what consumers read.
        out.push(match &m.provider_turn {
            Some(turn) => Msg::Turn {
                shape: turn.wire_shape.clone(),
                wire: turn.wire.clone(),
                fallback: Box::new(projected),
            },
            None => projected,
        });
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
    if config.chat_wire_shape == "ChatGoogle" {
        transform_google_parts(body, msgs, request, config);
    } else if config.chat_wire_shape == "ChatResponsesOpenAI" {
        transform_responses_input(body, msgs, request, config);
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
    body.insert(
        "messages".into(),
        Value::Array(build_flat_message_array(msgs, request, config)),
    );
}

/// Builds the OpenAI Responses envelope (ADR-055): the SAME flat {role, content}
/// array as Chat Completions, but under the "input" key instead of "messages"
/// (and POSTed to /v1/responses). The array shape is shared with
/// [`transform_flat_content`] via [`build_flat_message_array`], so the golden
/// witnesses that the only wire delta is the envelope key + endpoint.
fn transform_responses_input(
    body: &mut Map<String, Value>,
    msgs: &[Msg],
    request: &Request,
    config: &ProviderSpec,
) {
    body.insert(
        "input".into(),
        Value::Array(build_flat_message_array(msgs, request, config)),
    );
}

/// Renders one canonical message as a flat-envelope entry — the reconstruction
/// path, unchanged from before ADR-085 and still what every caller-authored
/// turn takes.
fn flat_projected_entry(m: &Msg, config: &ProviderSpec, bedrock: bool) -> Value {
    match m {
        Msg::Result(result) => tool_result_message(config, result),
        Msg::Calls(calls) => tool_call_message(config, calls),
        Msg::Text { role, text } => {
            if bedrock {
                json!({
                    "role": map_role(role, config),
                    "content": [{"text": text}],
                })
            } else {
                json!({
                    "role": map_role(role, config),
                    "content": text,
                })
            }
        }
        // A payload the splice could not place falls back to its projection.
        // `resolve_turns` should already have unwrapped anything unplaceable —
        // this arm is what makes "should" not load-bearing, and it is the arm
        // Bedrock takes: ChatBedrock declares assistantTurnUnanchored
        // rather than a position (ADR-085 OQ-5), so there is no container to
        // splice into. When OQ-5 anchors Converse this becomes a real splice.
        Msg::Turn { fallback, .. } => flat_projected_entry(fallback, config, bedrock),
    }
}

/// Appends a captured assistant turn to a flat-envelope array in whatever
/// container that wire family expects, returning false when the payload cannot
/// be placed so the caller reconstructs instead.
///
/// The three families disagree on what `assistantTurnPath` even points at,
/// which is why this cannot be one push:
///
///   - `ChatOpenAI`    `choices[0].message` -> an assistant message object
///   - `ChatAnthropic` `content`            -> the block ARRAY, with no message
///     object around it; the role wrapper below is llmkit's, the blocks are the
///     provider's
///   - `ChatResponses` `output`             -> an ITEM LIST that spreads across
///     N input entries rather than becoming one (ADR-085 OQ-1)
fn append_flat_replayed_turn(
    out: &mut Vec<Value>,
    shape: &str,
    wire: &str,
    config: &ProviderSpec,
    bedrock: bool,
) -> bool {
    if bedrock {
        return false;
    }
    let Ok(payload) = serde_json::from_str::<Value>(wire) else {
        return false;
    };
    match shape {
        "ChatAnthropic" => {
            out.push(json!({
                "role": map_role("assistant", config),
                "content": payload,
            }));
            true
        }
        "ChatResponsesOpenAI" => {
            let Value::Array(items) = payload else {
                return false;
            };
            out.extend(items);
            true
        }
        _ => {
            out.push(payload);
            true
        }
    }
}

/// Builds the shared flat message array used by both the Chat Completions
/// ("messages") and Responses ("input") envelopes.
fn build_flat_message_array(msgs: &[Msg], request: &Request, config: &ProviderSpec) -> Vec<Value> {
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
            if let Msg::Turn { shape, wire, .. } = m {
                if append_flat_replayed_turn(&mut messages, shape, wire, config, bedrock) {
                    continue;
                }
            }
            messages.push(flat_projected_entry(m, config, bedrock));
        }
    } else if let Some(user) = &request.user {
        if bedrock {
            let content = if request.images.is_empty() {
                json!([{"text": user}])
            } else {
                Value::Array(build_bedrock_content_parts(request))
            };
            messages.push(json!({
                "role": map_role("user", config),
                "content": content,
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

    messages
}

/// Renders one canonical message as a Google `contents` entry — the
/// reconstruction path. Mirrors [`flat_projected_entry`], including its final
/// arm: a payload the splice could not place degrades to its own projection
/// rather than panicking out of a public request build.
fn google_projected_entry(
    m: &Msg,
    config: &ProviderSpec,
    id_to_name: &mut HashMap<String, String>,
) -> Value {
    match m {
        Msg::Result(result) => {
            let resolved = match id_to_name.get(&result.tool_use_id) {
                Some(name) => ToolResult {
                    tool_use_id: name.clone(),
                    content: result.content.clone(),
                },
                None => result.clone(),
            };
            tool_result_message(config, &resolved)
        }
        Msg::Calls(calls) => {
            for call in calls {
                id_to_name.insert(call.id.clone(), call.name.clone());
            }
            tool_call_message(config, calls)
        }
        Msg::Text { role, text } => json!({
            "role": map_role(role, config),
            "parts": [{"text": text}],
        }),
        Msg::Turn { fallback, .. } => google_projected_entry(fallback, config, id_to_name),
    }
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
            // A replayed Google turn is candidates[0].content verbatim — the
            // same {role, parts} object the contents array takes, so it drops
            // straight in. It still has to feed id_to_name below, because a
            // LATER tool result is matched by name against calls made on this
            // turn; that lookup reads the canonical projection, which the
            // fallback still carries even when the payload is what gets sent.
            if let Msg::Turn {
                shape,
                wire,
                fallback,
            } = m
            {
                if shape == "ChatGoogle" {
                    if let Msg::Calls(calls) = &**fallback {
                        for call in calls {
                            id_to_name.insert(call.id.clone(), call.name.clone());
                        }
                    }
                    if let Ok(payload) = serde_json::from_str::<Value>(wire) {
                        contents.push(payload);
                        continue;
                    }
                }
            }
            contents.push(google_projected_entry(m, config, &mut id_to_name));
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

// Builds a Bedrock Converse content array with image blocks (ADR-060). Each
// image emits {image:{format,source:{bytes}}}; the prompt text follows as a
// trailing {text} block, preserving caller order among images. Mirror of
// go/transforms.go buildBedrockContentParts.
fn build_bedrock_content_parts(request: &Request) -> Vec<Value> {
    let mut parts = Vec::new();
    for image in &request.images {
        let (mut mime_type, data) = parse_data_uri(&image.url);
        if mime_type.is_empty() {
            mime_type = image.mime_type.clone();
        }
        parts.push(json!({
            "image": {
                "format": bedrock_image_format(&mime_type),
                "source": {"bytes": data},
            }
        }));
    }
    if let Some(user) = &request.user {
        parts.push(json!({"text": user}));
    }
    parts
}

// Derives the Converse `format` token from a MIME type (image/png -> "png").
fn bedrock_image_format(mime_type: &str) -> &str {
    match mime_type.rfind('/') {
        Some(i) => &mime_type[i + 1..],
        None => mime_type,
    }
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

/// Builds a tool result for the OpenAI Responses protocol (ADR-055). Responses
/// does not accept the Chat Completions tool message: `input[]` entries carry
/// only the roles assistant/system/developer/user, and a tool result is a
/// top-level typed item instead.
///
/// LIVE-ANCHORED 2026-08-13: the Chat Completions shape this used to fall
/// through to is rejected 400 invalid_value on `input[3]`; the shape below
/// returns 200. Witnessed by replay-responses-openai-reasoning.json.
fn transform_responses_tool_result_msg(result: &ToolResult) -> Value {
    json!({
        "type": "function_call_output",
        "call_id": result.tool_use_id,
        "output": result.content,
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


































































































































