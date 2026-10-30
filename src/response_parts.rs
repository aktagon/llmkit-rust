//! The response's assistant turn as ordered parts (owner decision 1,
//! HANDOFF-105 F4). One reader and one writer, both driven by the generated
//! `response_text_config` and `tool_call_response_config` tables, replace the
//! four handwritten tool-call extractors (owner decision 2).

use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::paths::{extract_string_path, navigate_path, set_wire_path};
use crate::providers::generated::request::{tool_call_config, ToolCallDef};
use crate::providers::generated::response::{
    tool_call_response_config, ResponseTextConfig, ToolCallResponseConfig,
};
use crate::structs::{Part, Response, ToolCall};
use crate::{response_text_config, response_text_path, ProviderName};

impl Response {
    /// The response text: the text parts joined with no separator, in order.
    /// A block-array wire splits one reply into consecutive text blocks
    /// (Anthropic around a citation), so the blocks join as they are. Empty
    /// when the response carries no text, such as a turn that only calls a
    /// tool; the finish reason tells why.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The tool calls the model asked for, in the order it sent them, not
    /// run. Empty when the model called no tool.
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::ToolCall(call) => Some(call.clone()),
                _ => None,
            })
            .collect()
    }
}

/// The generated marker semantics for one array element: no marker path
/// matches every element; a marker path with no value matches when the key is
/// PRESENT; both set match when the key EQUALS the value.
fn block_matches(block: &Value, marker_path: &str, marker_value: &str) -> bool {
    if marker_path.is_empty() {
        return true;
    }
    match block.get(marker_path) {
        None => false,
        Some(marker) => marker_value.is_empty() || marker.as_str() == Some(marker_value),
    }
}

/// Reads a Chat Completions-family body's assistant turn as ordered parts. A
/// shape whose text is a scalar (ChatOpenAI) yields its text first, then its
/// tool calls. A block-array shape yields its text and tool-call blocks in
/// array order, because the two configs name the same array.
///
/// A tool-call block the caller could not run is `Error::Validation` on
/// `tool_calls`, never dropped (HANDOFF-105 F5): a block of a provider with no
/// tool-call config, and, in an array of tool calls only (one no text config
/// names, ChatOpenAI), an element that fails the marker, such as an OpenAI
/// call of type "custom". In an array shared with text the marker selects, so
/// a block that matches neither config (a thinking block) is not a part.
pub(crate) fn decode_chat_parts(
    raw: &Value,
    provider: ProviderName,
    chat_wire_shape: &str,
) -> Result<Vec<Part>, Error> {
    let text_cfg = response_text_config(chat_wire_shape);
    let tool_cfg = tool_call_response_config(chat_wire_shape);
    let tool_def = tool_call_config(provider);

    let mut parts = Vec::new();
    if text_cfg.is_none() {
        let text = extract_string_path(raw, response_text_path(provider));
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
    }
    for path in block_array_paths(text_cfg, tool_cfg) {
        let Some(Value::Array(blocks)) = navigate_path(raw, path) else {
            continue;
        };
        let text_here = text_cfg.filter(|cfg| cfg.blocks_path == path);
        let tool_here = tool_cfg.filter(|cfg| cfg.blocks_path == path);
        let tools_only = tool_here.is_some() && text_here.is_none();
        for (i, block) in blocks.iter().enumerate() {
            let object = block.is_object();
            if let Some(cfg) = text_here
                .filter(|cfg| object && block_matches(block, cfg.marker_path, cfg.marker_value))
            {
                let text = extract_string_path(block, cfg.value_path);
                if !text.is_empty() {
                    parts.push(Part::Text(text));
                }
            } else if let Some(cfg) = tool_here {
                let marked = object && block_matches(block, cfg.marker_path, cfg.marker_value);
                if tools_only && !marked {
                    return Err(other_tool_call_type(path, i, block, &cfg));
                }
                if marked {
                    let Some(def) = tool_def else {
                        return Err(no_tool_call_config_error(provider, chat_wire_shape));
                    };
                    parts.push(Part::ToolCall(read_tool_call(block, &cfg, def)?));
                }
            }
        }
    }
    Ok(parts)
}

/// The rejection for an element of a tool-call-only array that is not a tool
/// call of the kind the config marks (owner decision 1, HANDOFF-105 F5): it
/// is a call the caller cannot run, so it is not dropped.
fn other_tool_call_type(
    path: &str,
    i: usize,
    block: &Value,
    cfg: &ToolCallResponseConfig,
) -> Error {
    if !block.is_object() {
        return malformed_tool_call(format!("{path}[{i}] is not an object"));
    }
    malformed_tool_call(format!(
        "{path}[{i}] has {} {}, want {:?}",
        cfg.marker_path,
        block.get(cfg.marker_path).unwrap_or(&Value::Null),
        cfg.marker_value
    ))
}

/// The arrays the two configs scan, each once, text first.
fn block_array_paths(
    text_cfg: Option<ResponseTextConfig>,
    tool_cfg: Option<ToolCallResponseConfig>,
) -> Vec<&'static str> {
    let mut paths = Vec::new();
    if let Some(cfg) = text_cfg {
        paths.push(cfg.blocks_path);
    }
    if let Some(cfg) = tool_cfg {
        if text_cfg.map(|text| text.blocks_path) != Some(cfg.blocks_path) {
            paths.push(cfg.blocks_path);
        }
    }
    paths
}

/// Reads one tool-call block. An empty `id_path` means the family carries no
/// id on the wire; the id is the tool name when the provider's
/// `toolCallIDSource` says so (Google). The arguments are decoded per the
/// provider's args format: a JSON string (`json_string`) or a JSON value
/// (`map`). Absent arguments leave `input` as `None`. A block with no tool
/// name, or arguments that do not decode, is a validation error on field
/// `tool_calls`: it is not a call the caller can run.
fn read_tool_call(
    block: &Value,
    cfg: &ToolCallResponseConfig,
    def: &ToolCallDef,
) -> Result<ToolCall, Error> {
    let name = extract_string_path(block, cfg.name_path);
    if name.is_empty() {
        return Err(malformed_tool_call(format!(
            "no tool name at {}",
            cfg.name_path
        )));
    }
    let id = if !cfg.id_path.is_empty() {
        extract_string_path(block, cfg.id_path)
    } else if def.id_source == "function_name" {
        name.clone()
    } else {
        String::new()
    };
    let input = match navigate_path(block, cfg.args_path) {
        None | Some(Value::Null) => None,
        Some(args) if def.args_format == "json_string" => {
            let Some(text) = args.as_str() else {
                return Err(malformed_tool_call(format!(
                    "{name:?}: arguments at {} are not a JSON string",
                    cfg.args_path
                )));
            };
            Some(serde_json::from_str::<Value>(text).map_err(|err| {
                malformed_tool_call(format!("{name:?}: arguments at {}: {err}", cfg.args_path))
            })?)
        }
        Some(args) => Some(args.clone()),
    };
    Ok(ToolCall { id, name, input })
}

/// A tool-call block that is not a call the caller can run is
/// `Error::Validation` on field `tool_calls`, the field `encode_response`
/// rejects with too.
fn malformed_tool_call(message: String) -> Error {
    Error::Validation {
        field: "tool_calls".into(),
        message: format!("decode tool call {message}"),
    }
}

/// Writes the parts into a Chat Completions-family body, in part order,
/// through the same configs `decode_chat_parts` reads. Text and tool calls
/// that share a block array (Anthropic, Google, Bedrock) go into it in the
/// parts' order; a scalar-text shape (ChatOpenAI) writes the text at its path
/// and the tool calls into their own array. A tool call on a shape or provider
/// with no tool-call config is `Error::Validation` on field `tool_calls`: there
/// is nowhere to write it, and dropping it would lose it.
pub(crate) fn encode_chat_parts(
    raw: &mut Value,
    provider: ProviderName,
    chat_wire_shape: &str,
    response: &Response,
) -> Result<(), Error> {
    let text_cfg = response_text_config(chat_wire_shape);
    let tool_cfg = tool_call_response_config(chat_wire_shape);
    let tool_def = tool_call_config(provider);

    let mut arrays: Vec<(&'static str, Vec<Value>)> = Vec::new();
    let mut push =
        |path: &'static str, block: Value| match arrays.iter_mut().find(|(p, _)| *p == path) {
            Some((_, blocks)) => blocks.push(block),
            None => arrays.push((path, vec![block])),
        };
    for part in &response.parts {
        match part {
            Part::ToolCall(call) => {
                let (Some(cfg), Some(def)) = (tool_cfg, tool_def) else {
                    return Err(no_tool_call_config_error(provider, chat_wire_shape));
                };
                push(cfg.blocks_path, write_tool_call(call, &cfg, def)?);
            }
            Part::Text(text) => {
                if let Some(cfg) = text_cfg {
                    let mut block = Value::Object(Map::new());
                    if !cfg.marker_value.is_empty() {
                        set_wire_path(&mut block, cfg.marker_path, json!(cfg.marker_value));
                    }
                    set_wire_path(&mut block, cfg.value_path, json!(text));
                    push(cfg.blocks_path, block);
                }
            }
            _ => {}
        }
    }
    if text_cfg.is_none() {
        set_wire_path(raw, response_text_path(provider), json!(response.text()));
    }
    for (path, blocks) in arrays {
        set_wire_path(raw, path, Value::Array(blocks));
    }
    Ok(())
}

/// Renders one tool call as a block of the shape. The marker is written when
/// it is a value (Anthropic type=tool_use); a presence marker is the key the
/// name path starts with, so writing the name writes it (the generator checks
/// this). No id is written where the shape has no id path (Google):
/// `exceptGoogleToolCallID` is OneWay (owner decision 3).
fn write_tool_call(
    call: &ToolCall,
    cfg: &ToolCallResponseConfig,
    def: &ToolCallDef,
) -> Result<Value, Error> {
    let mut block = Value::Object(Map::new());
    if !cfg.marker_value.is_empty() {
        set_wire_path(&mut block, cfg.marker_path, json!(cfg.marker_value));
    }
    set_wire_path(&mut block, cfg.id_path, json!(call.id));
    set_wire_path(&mut block, cfg.name_path, json!(call.name));
    if let Some(input) = call.input.as_ref().filter(|input| !input.is_null()) {
        let args = if def.args_format == "json_string" {
            json!(serde_json::to_string(input)?)
        } else {
            input.clone()
        };
        set_wire_path(&mut block, cfg.args_path, args);
    }
    Ok(block)
}

/// The rejection for a response with tool calls on a shape that declares no
/// `ToolCallResponseConfig` (ChatResponsesOpenAI), or for a provider with
/// no tool-call config.
pub(crate) fn no_tool_call_config_error(provider: ProviderName, chat_wire_shape: &str) -> Error {
    Error::Validation {
        field: "tool_calls".into(),
        message: format!(
            "provider {provider:?} has no tool-call response shape on chat wire shape {chat_wire_shape:?}"
        ),
    }
}
