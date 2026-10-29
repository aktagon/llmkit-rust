use std::collections::HashMap;

use base64::Engine;
use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::part_kinds::check_part_kinds;
use crate::paths::matching_blocks;
use crate::providers::generated::providers::ProviderSpec;
use crate::providers::generated::request::{system_placement, tool_call_config, SystemPlacement};
use crate::structs::Request;
use crate::structs::ToolDefinition;
use crate::structs::{MediaRef, Message, Part, ToolCall, ToolResult};

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
    tools: &[ToolDefinition],
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
// Internal message sum (ADR-026 PIPE-007/008, ADR-100)
// =============================================================================

/// The internal message representation. `Parts` is a validated turn: a role
/// and its parts, each part a kind the role may carry (ADR-100 MSG-002).
/// `Turn` wraps one with the provider's own bytes.
#[derive(Clone, Debug)]
pub(crate) enum Msg {
    /// A validated turn: its role and its parts, in order.
    Parts { role: String, parts: Vec<Part> },
    /// An assistant turn the provider itself serialized, replayed verbatim
    /// instead of rebuilt (ADR-085). It carries the turn it replaces so
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

/// The part kinds each role may carry (ADR-100 MSG-002).
const ROLE_PARTS: &[(&str, &[&str])] = &[
    ("user", &["text", "image", "file"]),
    ("assistant", &["text", "tool_call"]),
    ("tool", &["tool_result"]),
];

/// The one place a [`Request`] becomes the message list the transforms read:
/// the validated turns, the final prompt included (ADR-098 RQC-001).
pub(crate) fn request_msgs(request: &Request) -> Result<Vec<Msg>, Error> {
    to_internal(&request.messages)
}

/// Converts the public, untrusted [`Message`] slice into the internal sum.
/// This is the single validation boundary (PIPE-008): a part of a kind chat
/// does not accept (MSG-011), or a kind the role cannot carry (MSG-002), is
/// rejected here with a validation error that names it, not mis-serialized
/// downstream. Text, batch, stream and the Agent all send through here.
pub(crate) fn to_internal(messages: &[Message]) -> Result<Vec<Msg>, Error> {
    let mut out = Vec::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        check_part_kinds("chat", &m.parts).map_err(|err| match err {
            Error::Validation { message, .. } => Error::Validation {
                field: "messages".into(),
                message: format!("messages[{i}].parts: {message}"),
            },
            other => other,
        })?;
        let allowed = ROLE_PARTS
            .iter()
            .find(|(role, _)| *role == m.role)
            .map(|(_, kinds)| *kinds)
            .unwrap_or(&[]);
        for (j, part) in m.parts.iter().enumerate() {
            let kind = part.kind();
            if !allowed.contains(&kind) {
                return Err(Error::Validation {
                    field: "messages".into(),
                    message: format!(
                        "messages[{i}].parts[{j}]: {} turn does not accept {kind} parts",
                        m.role
                    ),
                });
            }
        }
        let projected = Msg::Parts {
            role: m.role.clone(),
            parts: m.parts.clone(),
        };
        // provider_turn is the same turn in the provider's own serialization.
        // When present it supersedes the parts on the wire while the parts stay
        // what consumers read.
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

/// Reports whether any turn carries a file part.
pub(crate) fn request_has_files(request: &Request) -> bool {
    request
        .messages
        .iter()
        .flat_map(|m| &m.parts)
        .any(|p| matches!(p, Part::File(_)))
}

/// Joins a turn's text parts with "\n", the join the Text builder uses for
/// its final turn (ADR-100 MSG-003, MSG-004).
fn joined_text(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Returns the text of a turn that is at most one text part, which the flat
/// envelopes send as a content string rather than a block list. A turn with
/// no parts sends "" as before ADR-100.
fn plain_text(parts: &[Part]) -> Option<&str> {
    match parts {
        [] => Some(""),
        [Part::Text(t)] if !t.is_empty() => Some(t),
        _ => None,
    }
}

/// A turn's tool call parts, in order.
fn tool_calls_of(parts: &[Part]) -> Vec<&ToolCall> {
    parts
        .iter()
        .filter_map(|p| match p {
            Part::ToolCall(call) => Some(call),
            _ => None,
        })
        .collect()
}

fn base64_of(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Renders inline image bytes as the base64 data URI OpenAI takes.
fn data_uri(image: &MediaRef) -> String {
    format!(
        "data:{};base64,{}",
        image.mime_type,
        base64_of(&image.bytes)
    )
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
        transform_google_parts(body, msgs, config);
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

/// The turn a message is rebuilt from: its own parts, or the parts a replayed
/// turn carries as its fallback.
fn projected_parts(m: &Msg) -> (&str, &[Part]) {
    match m {
        Msg::Parts { role, parts } => (role, parts),
        Msg::Turn { fallback, .. } => projected_parts(fallback),
    }
}

/// Renders one canonical turn as flat-envelope entries — the reconstruction
/// path, and the counterpart of [`append_flat_replayed_turn`]. Both deal in
/// LISTS for the same reason: one turn is not always one wire entry. A Chat
/// Completions or Responses tool turn is one entry per result, and a
/// Responses assistant turn is one entry per part.
fn flat_projected_entries(role: &str, parts: &[Part], config: &ProviderSpec) -> Vec<Value> {
    let anthropic = config.chat_wire_shape == "ChatAnthropic";
    match role {
        "tool" => flat_tool_turn(parts, config),
        "assistant" if anthropic => vec![json!({
            "role": map_role("assistant", config),
            "content": anthropic_content(parts),
        })],
        "assistant" if config.chat_wire_shape == "ChatResponsesOpenAI" => {
            responses_assistant_turn(parts, config)
        }
        "assistant" => vec![openai_assistant_turn(parts, config)],
        _ if anthropic => vec![json!({
            "role": map_role(role, config),
            "content": anthropic_content(parts),
        })],
        _ => vec![json!({
            "role": map_role(role, config),
            "content": openai_content(parts),
        })],
    }
}

/// Renders a tool turn. Anthropic puts every result in one user message;
/// Chat Completions and Responses take one entry per result (ADR-100 MSG-009).
fn flat_tool_turn(parts: &[Part], config: &ProviderSpec) -> Vec<Value> {
    let results = parts.iter().filter_map(|p| match p {
        Part::ToolResult(result) => Some(result),
        _ => None,
    });
    if config.chat_wire_shape == "ChatAnthropic" {
        let blocks: Vec<Value> = results.map(anthropic_tool_result_block).collect();
        return vec![json!({"role": "user", "content": blocks})];
    }
    if config.chat_wire_shape == "ChatResponsesOpenAI" {
        return results.map(responses_tool_result).collect();
    }
    results.map(openai_tool_result).collect()
}

/// Renders an assistant turn for Chat Completions, which has no order between
/// text and tool calls: the text parts join into content and the tool call
/// parts go into tool_calls (ADR-100 MSG-003). A turn with only tool calls
/// carries no content key, as before ADR-100.
fn openai_assistant_turn(parts: &[Part], config: &ProviderSpec) -> Value {
    let mut entry = Map::new();
    entry.insert("role".into(), Value::String(map_role("assistant", config)));
    let calls = tool_calls_of(parts);
    if calls.is_empty() || calls.len() < parts.len() {
        entry.insert("content".into(), Value::String(joined_text(parts)));
    }
    if !calls.is_empty() {
        entry.insert(
            "tool_calls".into(),
            Value::Array(calls.into_iter().map(openai_tool_call).collect()),
        );
    }
    Value::Object(entry)
}

/// Renders an assistant turn for Responses, which keeps order: each text part
/// is an assistant message entry and each tool call its own function_call item
/// (ADR-055, BUG-050).
fn responses_assistant_turn(parts: &[Part], config: &ProviderSpec) -> Vec<Value> {
    if parts.is_empty() {
        return vec![json!({"role": map_role("assistant", config), "content": ""})];
    }
    parts
        .iter()
        .map(|p| match p {
            Part::ToolCall(call) => responses_function_call(call),
            Part::Text(text) => json!({"role": map_role("assistant", config), "content": text}),
            _ => unreachable!("to_internal admits only text and tool_call on an assistant turn"),
        })
        .collect()
}

/// Renders a user turn's content for Chat Completions and Responses: a string
/// for at most one text part, else blocks in part order. The image detail
/// stays "auto", the value every golden carries (MSG-015).
fn openai_content(parts: &[Part]) -> Value {
    if let Some(text) = plain_text(parts) {
        return Value::String(text.to_string());
    }
    Value::Array(
        parts
            .iter()
            .map(|p| match p {
                Part::File(file) => json!({"type": "file", "file": {"file_id": file.id}}),
                Part::Image(image) => json!({
                    "type": "image_url",
                    "image_url": {"url": data_uri(image), "detail": "auto"},
                }),
                Part::Text(text) => json!({"type": "text", "text": text}),
                _ => unreachable!("to_internal admits only text, image and file on a user turn"),
            })
            .collect(),
    )
}

/// Renders a user or assistant turn's content for Anthropic: a string for at
/// most one text part, else blocks in part order.
fn anthropic_content(parts: &[Part]) -> Value {
    if let Some(text) = plain_text(parts) {
        return Value::String(text.to_string());
    }
    Value::Array(
        parts
            .iter()
            .map(|p| match p {
                Part::File(file) => json!({
                    "type": "document",
                    "source": {"type": "file", "file_id": file.id},
                }),
                Part::Image(image) => json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": image.mime_type,
                        "data": base64_of(&image.bytes),
                    },
                }),
                Part::ToolCall(call) => anthropic_tool_use_block(call),
                Part::Text(text) => json!({"type": "text", "text": text}),
                _ => unreachable!("to_internal admits no tool_result on a user or assistant turn"),
            })
            .collect(),
    )
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
) -> bool {
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
/// ("messages") and Responses ("input") envelopes, and by Bedrock Converse.
fn build_flat_message_array(msgs: &[Msg], request: &Request, config: &ProviderSpec) -> Vec<Value> {
    let bedrock = config.chat_wire_shape == "ChatBedrock";
    let mut messages = Vec::new();

    if matches!(
        system_placement(config.name),
        SystemPlacement::MessageInArray
    ) && !request.system.is_empty()
    {
        messages.push(json!({
            "role": map_role("system", config),
            "content": request.system,
        }));
    }

    if bedrock {
        bedrock_messages(&mut messages, msgs, config);
        return messages;
    }

    for m in msgs {
        if let Msg::Turn { shape, wire, .. } = m {
            if append_flat_replayed_turn(&mut messages, shape, wire, config) {
                continue;
            }
        }
        // A payload the splice could not place falls back to its parts.
        // `resolve_turns` should already have unwrapped anything unplaceable;
        // this keeps a public request build from failing when it did not.
        let (role, parts) = projected_parts(m);
        messages.extend(flat_projected_entries(role, parts, config));
    }

    messages
}

/// Appends the Bedrock Converse messages. Bedrock never replays:
/// ChatBedrock declares assistantTurnUnanchored rather than a
/// position (ADR-085 OQ-5), so there is no container to splice into and every
/// turn is rebuilt from its parts. When OQ-5 anchors Converse this becomes a
/// real splice.
fn bedrock_messages(out: &mut Vec<Value>, msgs: &[Msg], config: &ProviderSpec) {
    let mut prev_result = false;
    for m in msgs {
        let (role, parts) = projected_parts(m);
        let is_result = role == "tool";
        let content = bedrock_content(parts);
        // Converse wants every toolResult answering one assistant turn inside
        // ONE user message; one message per result is a 400 (live,
        // 2026-10-15). A tool turn that follows a tool turn joins the message
        // the first one opened.
        if is_result && prev_result {
            if let Some(blocks) = out
                .last_mut()
                .and_then(|open| open.get_mut("content"))
                .and_then(Value::as_array_mut)
            {
                blocks.extend(content);
                continue;
            }
        }
        prev_result = is_result;
        let role = if is_result {
            "user".to_string()
        } else {
            map_role(role, config)
        };
        out.push(json!({"role": role, "content": content}));
    }
}

/// Renders a turn's parts as a Converse content array, in part order
/// (ADR-060, ADR-100 MSG-003). Each image is {image:{format,source:{bytes}}}.
/// Converse has no block for an uploaded file id, so a file part is not sent,
/// as before ADR-100. A turn with no parts sends one empty text block.
fn bedrock_content(parts: &[Part]) -> Vec<Value> {
    if parts.is_empty() {
        return vec![json!({"text": ""})];
    }
    parts
        .iter()
        .filter_map(|p| match p {
            Part::File(_) => None,
            Part::Image(image) => Some(json!({
                "image": {
                    "format": bedrock_image_format(&image.mime_type),
                    "source": {"bytes": base64_of(&image.bytes)},
                }
            })),
            Part::ToolCall(call) => Some(bedrock_tool_use_block(call)),
            Part::ToolResult(result) => Some(bedrock_tool_result_block(result)),
            Part::Text(text) => Some(json!({"text": text})),
            _ => unreachable!("to_internal admits only chat part kinds"),
        })
        .collect()
}

fn transform_google_parts(body: &mut Map<String, Value>, msgs: &[Msg], config: &ProviderSpec) {
    let mut contents = Vec::new();

    // Google's wire identifies a tool result by the function NAME, but the
    // universal ToolResult carries only tool_use_id. Recover id->name from the
    // tool call parts, which always precede their result in a valid history,
    // and send the name (the agent path is unaffected: its extractor sets
    // id==name). An unmatched id passes through unchanged.
    let mut id_to_name: HashMap<String, String> = HashMap::new();
    for m in msgs {
        let (role, parts) = projected_parts(m);
        for call in tool_calls_of(parts) {
            id_to_name.insert(call.id.clone(), call.name.clone());
        }
        // A replayed Google turn is candidates[0].content verbatim — the same
        // {role, parts} object the contents array takes, so it drops straight
        // in. Its calls still fed id_to_name above, because a LATER tool result
        // is matched by name against them.
        if let Msg::Turn { shape, wire, .. } = m {
            if shape == "ChatGoogle" {
                if let Ok(payload) = serde_json::from_str::<Value>(wire) {
                    contents.push(payload);
                    continue;
                }
            }
        }
        let role = if role == "tool" {
            "user".to_string()
        } else {
            map_role(role, config)
        };
        contents.push(json!({
            "role": role,
            "parts": google_parts(parts, &id_to_name),
        }));
    }

    body.insert("contents".into(), Value::Array(contents));
}

/// Renders a turn's parts for Google, in part order. A turn with no parts
/// sends one empty text part, as before ADR-100.
fn google_parts(parts: &[Part], id_to_name: &HashMap<String, String>) -> Vec<Value> {
    if parts.is_empty() {
        return vec![json!({"text": ""})];
    }
    parts
        .iter()
        .map(|p| match p {
            Part::File(file) => json!({
                "file_data": {"file_uri": file.uri, "mime_type": file.mime_type}
            }),
            Part::Image(image) => json!({
                "inline_data": {"mime_type": image.mime_type, "data": base64_of(&image.bytes)}
            }),
            Part::ToolCall(call) => google_function_call(call),
            Part::ToolResult(result) => {
                let name = id_to_name
                    .get(&result.tool_use_id)
                    .unwrap_or(&result.tool_use_id);
                google_function_response(name, &result.content)
            }
            Part::Text(text) => json!({"text": text}),
            _ => unreachable!("to_internal admits only chat part kinds"),
        })
        .collect()
}

// Derives the Converse `format` token from a MIME type (image/png -> "png").
fn bedrock_image_format(mime_type: &str) -> &str {
    match mime_type.rfind('/') {
        Some(i) => &mime_type[i + 1..],
        None => mime_type,
    }
}

fn transform_openai_functions(body: &mut Map<String, Value>, tools: &[ToolDefinition]) {
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

fn transform_anthropic_tools(body: &mut Map<String, Value>, tools: &[ToolDefinition]) {
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
    tools: &[ToolDefinition],
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

fn transform_bedrock_tool_defs(body: &mut Map<String, Value>, tools: &[ToolDefinition]) {
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

/// One Chat Completions tool_calls entry, with the arguments as a JSON string.
fn openai_tool_call(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": serde_json::to_string(&tool_call_input_value(call)).unwrap_or_else(|_| "{}".into()),
        }
    })
}

/// One tool call for the OpenAI Responses protocol (ADR-055). Responses has
/// no assistant message envelope for tool calls: each call is its own
/// top-level `input[]` item, correlated to its output by `call_id` rather than
/// by position in a `tool_calls` array.
///
/// LIVE-ANCHORED 2026-08-13 (two arms against /v1/responses, two parallel calls
/// in each): the Chat Completions shape this used to fall through to is rejected
/// 400 `missing_required_parameter` on `input[1].content` — Responses accepts
/// the assistant role, then demands the content a `tool_calls`-only message has
/// not got; the shape below returns 200 "completed" with both results consumed.
fn responses_function_call(call: &ToolCall) -> Value {
    json!({
        "type": "function_call",
        "call_id": call.id,
        "name": call.name,
        "arguments": serde_json::to_string(&tool_call_input_value(call)).unwrap_or_else(|_| "{}".into()),
    })
}

fn anthropic_tool_use_block(call: &ToolCall) -> Value {
    json!({
        "type": "tool_use",
        "id": call.id,
        "name": call.name,
        "input": tool_call_input_value(call),
    })
}

fn google_function_call(call: &ToolCall) -> Value {
    json!({
        "functionCall": {
            "name": call.name,
            "args": tool_call_input_value(call),
        }
    })
}

fn bedrock_tool_use_block(call: &ToolCall) -> Value {
    json!({
        "toolUse": {
            "toolUseId": call.id,
            "name": call.name,
            "input": tool_call_input_value(call),
        }
    })
}

fn openai_tool_result(result: &ToolResult) -> Value {
    json!({
        "role": "tool",
        "content": result.content,
        "tool_call_id": result.tool_use_id,
    })
}

/// A tool result for the OpenAI Responses protocol (ADR-055). Responses does
/// not accept the Chat Completions tool message: `input[]` entries carry only
/// the roles assistant/system/developer/user, and a tool result is a top-level
/// typed item instead.
///
/// LIVE-ANCHORED 2026-08-13: the Chat Completions shape this used to fall
/// through to is rejected 400 invalid_value on `input[3]`; the shape below
/// returns 200. Witnessed by replay-responses-openai-reasoning.json.
fn responses_tool_result(result: &ToolResult) -> Value {
    json!({
        "type": "function_call_output",
        "call_id": result.tool_use_id,
        "output": result.content,
    })
}

fn anthropic_tool_result_block(result: &ToolResult) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": result.tool_use_id,
        "content": result.content,
    })
}

fn google_function_response(name: &str, content: &str) -> Value {
    json!({
        "functionResponse": {
            "name": name,
            "response": {"result": content},
        }
    })
}

fn bedrock_tool_result_block(result: &ToolResult) -> Value {
    json!({
        "toolResult": {
            "toolUseId": result.tool_use_id,
            "content": [{"text": result.content}],
        }
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

// The N=1 proof for `matching_blocks`: the SAME call the text reader makes,
// with a different marker value. Before BUG-053 these were two hand-rolled
// scans over one array that happened to agree; agreement by coincidence is what
// let text extraction break on thinking blocks while tool-call extraction,
// scanning the very same array, kept working.
fn extract_anthropic_tool_calls(raw: &Value) -> Vec<ToolCall> {
    matching_blocks(raw, "content", "type", "tool_use")
        .into_iter()
        .map(|block| ToolCall {
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
































































































































































