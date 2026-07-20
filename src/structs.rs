//

use crate::types::{Capability, Provider, Usage};
use std::collections::HashMap;

///
#
pub struct AudioData {
    ///
    pub mime_type: String,

    ///
    pub bytes: Vec<u8>,
}

///
#
pub struct BatchHandle {
    ///
    pub id: String,

    ///
    pub provider: Provider,

    ///
    pub raw: bool,
}

///
#
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
#
pub struct ImageData {
    ///
    pub mime_type: String,

    ///
    pub bytes: Vec<u8>,
}

///
#
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
#
pub struct LiveResult {
    ///
    pub models: Vec<ModelInfo>,

    ///
    pub errors: HashMap<String, ProviderError>,
}

///
#
pub struct MediaRef {
    ///
    pub mime_type: String,

    ///
    pub bytes: Vec<u8>,
}

///
#
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
#
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
#
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
#
pub struct ProviderError {
    ///
    pub kind: String,

    ///
    pub message: String,
}

///
#
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
#
pub struct SpeechResponse {
    ///
    pub audio: AudioData,

    ///
    pub usage: Usage,

    ///
    pub finish_reason: String,
}

///
#
pub struct ToolCall {
    ///
    pub id: String,

    ///
    pub name: String,

    ///
    pub input: Option<serde_json::Value>,
}

///
#
pub struct ToolResult {
    ///
    pub tool_use_id: String,

    ///
    pub content: String,
}

///
#
pub struct TranscriptSegment {
    ///
    pub text: String,

    ///
    pub start: i64,

    ///
    pub end: i64,

    ///
    pub speaker: String,
}

///
#
pub struct TranscriptionHandle {
    ///
    pub id: String,

    ///
    pub provider: Provider,
}

///
#
pub struct TranscriptionResponse {
    ///
    pub text: String,

    ///
    pub segments: Vec<TranscriptSegment>,

    ///
    pub usage: Usage,
}

///
#
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
#
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
#
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
