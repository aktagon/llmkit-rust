//


use super::middleware::MiddlewareOp;

//
//
//

pub const TELEMETRY_SEMCONV_VERSION: &str = "1.29.0";
pub const TELEMETRY_TRACES_PATH: &str = "/v1/traces";
pub const TELEMETRY_ENDPOINT_REQUIRED: bool = true;
pub const TELEMETRY_CAPTURE_CONTENT_DEFAULT: bool = false;

//
pub const OTEL_ATTR_OP: &str = "gen_ai.operation.name"; // Event.op
pub const OTEL_ATTR_PROVIDER: &str = "gen_ai.system"; // Event.provider
pub const OTEL_ATTR_MODEL: &str = "gen_ai.request.model"; // Event.model
pub const OTEL_ATTR_ERR_TYPE: &str = "error.type"; // Event.err_type

//
pub const OTEL_USAGE_INPUT: &str = "gen_ai.usage.input_tokens";
pub const OTEL_USAGE_OUTPUT: &str = "gen_ai.usage.output_tokens";

///
///
pub fn telemetry_operation_name(op: MiddlewareOp) -> Option<&'static str> {
    match op {
        MiddlewareOp::LlmRequest => Some("chat"),
        MiddlewareOp::ToolCall => Some("execute_tool"),
        _ => None,
    }
}
