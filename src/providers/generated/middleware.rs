//


use std::collections::HashMap;
use serde_json::Value;

#
pub struct Usage {
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
    pub reasoning: i64,
    ///
    pub cost: f64,
}

#
pub enum MiddlewarePhase {
    #
    Pre,
    Post,
}

#
pub enum MiddlewareOp {
    #
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

#
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
