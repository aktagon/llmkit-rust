// Code generated — DO NOT EDIT.


use std::collections::HashMap;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub reasoning: i64,
    ///
    pub cost: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MiddlewarePhase {
    #[default]
    Pre,
    Post,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MiddlewareOp {
    #[default]
    LlmRequest,
    ToolCall,
    CacheCreate,
    Upload,
    BatchSubmit,
    ImageGeneration,
    MusicGeneration,
    VideoGeneration,
    ModelsList,
}

#[derive(Clone, Debug, Default)]
pub struct Event {
    ///
    pub op: MiddlewareOp,
    ///
    pub phase: MiddlewarePhase,
    ///
    pub provider: String,
    ///
    pub model: String,
    ///
    pub tool: String,
    ///
    pub args: HashMap<String, Value>,
    ///
    pub result: String,
    ///
    pub usage: Option<Usage>,
    ///
    pub err: Option<String>,
    ///
    pub err_type: String,
    ///
    pub duration: Option<std::time::Duration>,
}

//
//
