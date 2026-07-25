// Code generated — DO NOT EDIT.


use std::collections::HashMap;
use serde_json::Value;

/// A dimension is either reported — carrying a value that may legitimately be
/// zero — or not reported at all. The two are different claims: a provider
/// that says it used no cached tokens and a provider that never mentions
/// caching are not the same fact, and neither is a zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_write: Option<i64>,
    pub cache_read: Option<i64>,
    pub reasoning: Option<i64>,
    /// cost is the provider-reported request cost in USD (ADR-027). Not a TokenDimension — a distinct monetary field. Only OpenRouter (the request must opt in with usage: {include: true}) and xAI report it. Providers whose usageCostPath is empty never report cost, and the field is then ABSENT, not 0.0 — an unreported cost is not a free request (ADR-081 AVAIL-007).
    pub cost: Option<f64>,
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
    SpeechGeneration,
    Transcription,
}

#[derive(Clone, Debug, Default)]
pub struct Event {
    /// Always set.
    pub op: MiddlewareOp,
    /// Always set. Internal-only (drives pre/post dispatch); not an OTEL attribute.
    pub phase: MiddlewarePhase,
    /// Always set.
    pub provider: String,
    /// Always set.
    pub model: String,
    /// Only set when Op=tool_call. Internal-only.
    pub tool: String,
    /// Only set when Op=tool_call, Phase=pre. Mutation by middleware is observed by the tool. Internal-only.
    pub args: HashMap<String, Value>,
    /// Only set when Op=tool_call, Phase=post. Internal-only.
    pub result: String,
    /// Set for Op=llm_request, Phase=post. Expanded to gen_ai.usage.* via otelUsageAttribute on each TokenDimension, not a single attribute. Its optional dimensions are SHARED with the response the middleware observes (ADR-081): read them, do not write through them — mutating one rewrites what the caller receives.
    pub usage: Option<Usage>,
    /// Set in Phase=post when the operation failed. Human-readable; telemetry never re-parses it (ADR-071).
    pub err: Option<String>,
    /// Set in Phase=post when the operation failed: one of api_error | validation_error | error, stamped structurally from the typed error at the erasure seam (ADR-071). The OTLP builder reads this verbatim.
    pub err_type: String,
    /// Set in Phase=post. Internal-only (maps to span duration, not a gen_ai attribute).
    pub duration: Option<std::time::Duration>,
}

// MiddlewareFn signature is runtime-defined (needs a Context shape); see
// the handwritten layer once plan 009 runtime wiring lands for Rust.
