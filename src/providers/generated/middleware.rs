// Code generated — DO NOT EDIT.


#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
    pub cache_write: i64,
    pub cache_read: i64,
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
    pub args: std::collections::HashMap<String, serde_json::Value>,
    ///
    pub result: String,
    ///
    pub usage: Option<Usage>,
    ///
    pub err: Option<String>,
    ///
    pub duration: Option<std::time::Duration>,
}

//
//
