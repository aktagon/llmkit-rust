// Code generated — DO NOT EDIT.

use crate::types::{Capability, Provider, Usage};
use std::collections::HashMap;

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioData {
    ///
    pub mime_type: String,

    ///
    pub bytes: Vec<u8>,
}

///
#[derive(Clone, Debug, PartialEq)]
pub struct BatchHandle {
    ///
    pub id: String,

    ///
    pub provider: Provider,

    ///
    pub raw: bool,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct File {
    ///
    pub id: String,

    ///
    pub uri: String,

    ///
    pub mime_type: String,

    ///
    pub name: String,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageData {
    ///
    pub mime_type: String,

    ///
    pub bytes: Vec<u8>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageResponse {
    ///
    pub images: Vec<ImageData>,

    ///
    pub text: String,

    ///
    pub usage: Usage,

    ///
    pub finish_reason: String,

    ///
    pub finish_message: String,

    ///
    pub raw: Option<serde_json::Value>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveResult {
    ///
    pub models: Vec<ModelInfo>,

    ///
    pub errors: HashMap<String, ProviderError>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaRef {
    ///
    pub mime_type: String,

    ///
    pub bytes: Vec<u8>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    ///
    pub role: String,

    ///
    pub content: String,

    ///
    pub tool_calls: Vec<ToolCall>,

    ///
    pub tool_result: Option<ToolResult>,
}

///
#[derive(Clone, Debug, PartialEq)]
pub struct ModelInfo {
    ///
    pub id: String,

    ///
    pub provider: Provider,

    ///
    pub capabilities: Vec<Capability>,

    ///
    pub display_name: String,

    ///
    pub description: String,

    ///
    pub context_window: i64,

    ///
    pub max_output: i64,

    ///
    pub created: i64,

    ///
    pub raw: Option<serde_json::Value>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MusicResponse {
    ///
    pub audio: Vec<AudioData>,

    ///
    pub text: String,

    ///
    pub usage: Usage,

    ///
    pub finish_reason: String,

    ///
    pub finish_message: String,

    ///
    pub raw: Option<serde_json::Value>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderError {
    ///
    pub kind: String,

    ///
    pub message: String,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    ///
    pub text: String,

    ///
    pub usage: Usage,

    ///
    pub finish_reason: String,

    ///
    pub finish_message: String,

    ///
    pub raw: Option<serde_json::Value>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolCall {
    ///
    pub id: String,

    ///
    pub name: String,

    ///
    pub input: Option<serde_json::Value>,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolResult {
    ///
    pub tool_use_id: String,

    ///
    pub content: String,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoData {
    ///
    pub mime_type: String,

    ///
    pub url: String,

    ///
    pub bytes: Vec<u8>,

    ///
    pub duration_seconds: i64,
}

///
#[derive(Clone, Debug, PartialEq)]
pub struct VideoHandle {
    ///
    pub id: String,

    ///
    pub provider: Provider,

    ///
    pub raw: bool,

    ///
    pub model: String,
}

///
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoResponse {
    ///
    pub videos: Vec<VideoData>,

    ///
    pub usage: Usage,

    ///
    pub finish_reason: String,

    ///
    pub finish_message: String,

    ///
    pub raw: Option<serde_json::Value>,
}
