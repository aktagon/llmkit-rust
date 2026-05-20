// Code generated — DO NOT EDIT.

use crate::image::ImageData;
use crate::types::{Provider, Usage};

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
pub struct Message {
    ///
    pub role: String,

    ///
    pub content: String,
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
