use std::collections::HashMap;

use base64::Engine;
use serde_json::value::RawValue;
use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::json_text::json_text;
use crate::part_kinds::check_part_kinds;
use crate::providers::generated::providers::ProviderSpec;
use crate::providers::generated::request::{system_placement, tool_call_config, SystemPlacement};
use crate::providers::generated::response::tool_call_response_config;
use crate::response_parts::{check_json, write_tool_call};
use crate::structs::Request;
use crate::structs::ToolDefinition;
use crate::structs::{MediaRef, Message, Part, ToolCall, ToolResult};

/// The argument text a request sends for a tool call, on every chat shape,
/// Responses included (HANDOFF-105 F6): the caller's or the provider's text
/// as it is, or `{}` when there is no input, the input is null or the input
/// is empty, because the providers take an object (HANDOFF-105 F8, owner
/// decision 2).
fn request_tool_input(call: &ToolCall) -> String {
    match call.input.as_deref() {
        None | Some("") => "{}".to_owned(),
        Some(text) if text.trim() == "null" => "{}".to_owned(),
        Some(text) => text.to_owned(),
    }
}

/// One assistant tool call for the request body, written through the
/// generated `ToolCallResponseConfig` `encode_response` writes with
/// (owner decision 3, HANDOFF-105 F5), so the request and the response cannot
/// disagree. A call with no input goes out as an empty object.
fn request_tool_call(call: &ToolCall, config: &ProviderSpec) -> Value {
    let (Some(cfg), Some(def)) = (
        tool_call_response_config(config.chat_wire_shape),
        tool_call_config(config.name),
    ) else {
        unreachable!("check_request_tool_calls rejects a tool call with no tool-call config")
    };
    let call = ToolCall {
        input: Some(request_tool_input(call)),
        ..call.clone()
    };
    write_tool_call(&call, &cfg, def)
}

/// Rejects a request whose turns carry a tool call the provider has no
/// tool-call config to write, on a shape that writes tool calls from config
/// (every chat shape but ChatResponsesOpenAI, whose function_call item
/// `responses_function_call` writes). Dropping the call would send a
/// different conversation; `encode_response` rejects the same case.
pub(crate) fn check_request_tool_calls(msgs: &[Msg], config: &ProviderSpec) -> Result<(), Error> {
    if config.chat_wire_shape == "ChatResponsesOpenAI"
        || (tool_call_response_config(config.chat_wire_shape).is_some()
            && tool_call_config(config.name).is_some())
    {
        return Ok(());
    }
    for (i, m) in msgs.iter().enumerate() {
        let (_, parts) = projected_parts(m);
        if let Some(j) = parts.iter().position(|p| matches!(p, Part::ToolCall(_))) {
            return Err(Error::Validation {
                field: format!("messages[{i}].parts[{j}]"),
                message: format!(
                    "provider {:?} has no tool-call shape on chat wire shape {:?}",
                    config.name, config.chat_wire_shape
                ),
            });
        }
    }
    Ok(())
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
/// Rejects a part that sets no kind, a kind chat does not accept (MSG-011), or
/// a kind the role cannot carry (MSG-002). The field names the part,
/// `parts[j]`. The request encoder runs it on every turn, and
/// `encode_response` on the response's assistant turn. `written` is the input
/// text the caller's encoder writes for a tool call (`None`: it writes none),
/// so the check judges the text that goes out: a request writes
/// `request_tool_input`, which turns `""` into `{}` (HANDOFF-105 F8, owner
/// decision 2).
pub(crate) fn check_turn_parts(
    role: &str,
    parts: &[Part],
    written: fn(&ToolCall) -> Option<String>,
) -> Result<(), Error> {
    check_part_kinds("chat", parts)?;
    let allowed = ROLE_PARTS
        .iter()
        .find(|(r, _)| *r == role)
        .map(|(_, kinds)| *kinds)
        .unwrap_or(&[]);
    for (j, part) in parts.iter().enumerate() {
        let kind = part.kind();
        if !allowed.contains(&kind) {
            return Err(Error::Validation {
                field: format!("parts[{j}]"),
                message: format!("{role} turn does not accept {kind} parts"),
            });
        }
        if let Part::ToolCall(call) = part {
            if written(call).is_some_and(|input| check_json(&input).is_err()) {
                return Err(Error::Validation {
                    field: format!("parts[{j}]"),
                    message: format!("tool call {:?} input is not JSON", call.name),
                });
            }
        }
    }
    Ok(())
}

pub(crate) fn to_internal(messages: &[Message]) -> Result<Vec<Msg>, Error> {
    let mut out = Vec::with_capacity(messages.len());
    for (i, m) in messages.iter().enumerate() {
        check_turn_parts(&m.role, &m.parts, |call| Some(request_tool_input(call))).map_err(
            |err| match err {
                Error::Validation { field, message } => Error::Validation {
                    field: format!("messages[{i}].{field}"),
                    message,
                },
                other => other,
            },
        )?;
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

/// Joins a turn's text parts with no separator, the join `Response::text`
/// uses, so an assistant turn sent to Chat Completions carries the text the
/// response did (owner decision 4, HANDOFF-105 F5).
fn joined_text(parts: &[Part]) -> String {
    parts
        .iter()
        .filter_map(|p| match p {
            Part::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
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
            "content": anthropic_content(parts, config),
        })],
        "assistant" if config.chat_wire_shape == "ChatResponsesOpenAI" => {
            responses_assistant_turn(parts, config)
        }
        "assistant" => vec![openai_assistant_turn(parts, config)],
        _ if anthropic => vec![json!({
            "role": map_role(role, config),
            "content": anthropic_content(parts, config),
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
            Value::Array(
                calls
                    .into_iter()
                    .map(|call| request_tool_call(call, config))
                    .collect(),
            ),
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
fn anthropic_content(parts: &[Part], config: &ProviderSpec) -> Value {
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
                Part::ToolCall(call) => request_tool_call(call, config),
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
///
/// The provider's turn goes into the body as its text, a [`json_text`] node,
/// never as a parse of it: a `Value` sorts the keys of a tool call's
/// arguments, and every agent-loop tool-call turn is replayed (HANDOFF-105 F8).
fn append_flat_replayed_turn(
    out: &mut Vec<Value>,
    shape: &str,
    wire: &str,
    config: &ProviderSpec,
) -> bool {
    match shape {
        "ChatAnthropic" => {
            let Some(content) = replayed_json(wire) else {
                return false;
            };
            out.push(json!({
                "role": map_role("assistant", config),
                "content": content,
            }));
            true
        }
        "ChatResponsesOpenAI" => {
            let Ok(items) = serde_json::from_str::<Vec<&RawValue>>(wire) else {
                return false;
            };
            out.extend(items.into_iter().map(|item| json_text(item.get())));
            true
        }
        _ => {
            let Some(payload) = replayed_json(wire) else {
                return false;
            };
            out.push(payload);
            true
        }
    }
}

/// A captured turn's text as a [`json_text`] node, or `None` when it is not
/// one JSON value, so the caller reconstructs the turn instead.
fn replayed_json(wire: &str) -> Option<Value> {
    serde_json::from_str::<&RawValue>(wire).ok()?;
    Some(json_text(wire))
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
        let content = bedrock_content(parts, config);
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
fn bedrock_content(parts: &[Part], config: &ProviderSpec) -> Vec<Value> {
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
            Part::ToolCall(call) => Some(request_tool_call(call, config)),
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
                if let Some(payload) = replayed_json(wire) {
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
            "parts": google_parts(parts, &id_to_name, config),
        }));
    }

    body.insert("contents".into(), Value::Array(contents));
}

/// Renders a turn's parts for Google, in part order. A turn with no parts
/// sends one empty text part, as before ADR-100.
fn google_parts(
    parts: &[Part],
    id_to_name: &HashMap<String, String>,
    config: &ProviderSpec,
) -> Vec<Value> {
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
            Part::ToolCall(call) => request_tool_call(call, config),
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
        "arguments": request_tool_input(call),
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

fn map_role(role: &str, config: &ProviderSpec) -> String {
    config
        .role_mappings
        .iter()
        .find(|(from, _)| *from == role)
        .map(|(_, to)| (*to).to_string())
        .unwrap_or_else(|| role.to_string())
}





























































































































































