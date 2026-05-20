// Code generated — DO NOT EDIT.

use crate::types::{Capability, Provider, Usage};
use std::collections::HashMap;

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
    pub errors: HashMap<String, String>,
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
