use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::paths::{extract_string_path, opt_f64_path, opt_int_path, set_wire_path};
use crate::provider_turn::capture_provider_turn;
use crate::providers::generated::caching::cache_usage_paths;
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::response::{
    usage_cost_path, usage_cost_scale, wire_shape_finish_reason_fallback,
    wire_shape_finish_reasons, wire_shape_fixed_values, wire_shape_response_created_path,
    wire_shape_response_id_path, wire_shape_response_model_path, FinishReason, FinishReasonEntry,
    FixedJson,
};
use crate::response_parts::{decode_chat_parts, encode_chat_parts, no_tool_call_config_error};
use crate::structs::Part;
use crate::transforms::check_turn_parts;
use crate::{usage_paths, Provider, ProviderName, Response, Usage};

/// Extracts text + usage from a provider response body into the canonical
/// [`Response`]. `chat_wire_shape` is the EFFECTIVE wire shape for this request
/// (after `.protocol(...)` resolution, ADR-055): only `ChatResponsesOpenAI`
/// diverges (the `output[]` envelope); every other value uses the provider's
/// declared response paths.
///
/// Keyless, IO-free and pure (ADR-076 SYM-002): no `Client`, no credential, no
/// network, no clock — the subject is the generated [`ProviderName`], not the
/// credential-carrying `Provider`. The wire shape is required, not derived —
/// one provider can serve two chat protocols, and inferring it silently
/// mis-parses (SYM-003). This is the same function the chat send path calls
/// (SYM-004). It runs the served-shape check `encode_response` runs
/// (`served_chat_wire_shape`, HANDOFF-105 F4): a shape the provider does not
/// serve, or a provider with no chat shape, is `Error::Validation` on field
/// `chat_wire_shape`.
pub fn decode_response(
    provider: ProviderName,
    chat_wire_shape: &str,
    body: &str,
) -> Result<Response, Error> {
    let chat_wire_shape = served_chat_wire_shape(provider, chat_wire_shape)?;
    let raw: Value = serde_json::from_str(body)?;

    // ADR-085: capture the assistant turn as the provider serialized it, from
    // the ORIGINAL text rather than from `raw` — re-encoding the parsed value
    // would emit serde's rendering (object keys sorted), not the provider's.
    let provider_turn = capture_provider_turn(body, provider_config(provider), chat_wire_shape);

    if chat_wire_shape == "ChatResponsesOpenAI" {
        let mut response = parse_responses_envelope(&raw);
        response.provider_turn = provider_turn;
        response.chat_wire_shape = opt_string(chat_wire_shape.to_owned());
        return Ok(response);
    }

    let parts = decode_chat_parts(&raw, provider, chat_wire_shape)?;
    let (finish_reason, finish_message) = extract_finish_signal(&raw, provider);
    let cfg = provider_config(provider);

    Ok(Response {
        parts,
        usage: decode_usage(&raw, provider),
        finish_reason: decode_finish_reason(chat_wire_shape, finish_reason.as_deref(), Some(&raw)),
        provider_finish_reason: finish_reason,
        finish_message,
        raw: None,
        provider_turn,
        id: opt_string(extract_string_path(&raw, cfg.response_id_path)),
        model: opt_string(extract_string_path(&raw, cfg.response_model_path)),
        // A batch item outcome (ADR-099 CRF-015); the batch parser sets it.
        state: None,
        cause: None,
        created: opt_int_path(&raw, wire_shape_response_created_path(chat_wire_shape)),
        chat_wire_shape: opt_string(chat_wire_shape.to_owned()),
    })
}

/// Resolves the shape `decode_response` reads (HANDOFF-105 F4) and
/// `encode_response` writes: the named one, or the provider's default when
/// `chat_wire_shape` is empty. Callers that decode a body they know is Chat
/// Completions (batch result lines) pass "" for the default; ADR-055 requires
/// every provider's default protocol to be a Chat Completions family, so ""
/// never resolves into the Responses arm. A shape the provider
/// does not serve is `Error::Validation` on field `chat_wire_shape`, the name
/// `encode_request` uses (owner decision 2, HANDOFF-105 section 3b), and so is a provider that serves no chat shape at
/// all: writing provider paths into another family's body produces a body
/// neither family reads. Before F2 an unserved name fell back to the
/// provider's own shape here, so the caller got a body of a shape it did not
/// ask for.
fn served_chat_wire_shape(
    provider: ProviderName,
    chat_wire_shape: &str,
) -> Result<&'static str, Error> {
    let config = provider_config(provider);
    let shape = if chat_wire_shape.is_empty() {
        config.chat_wire_shape
    } else {
        chat_wire_shape
    };
    match crate::request::with_wire_shape(config, shape) {
        Some(served) if !served.chat_wire_shape.is_empty() => Ok(served.chat_wire_shape),
        _ => Err(Error::Validation {
            field: "chat_wire_shape".into(),
            message: format!(
                "provider {:?} does not serve chat wire shape {chat_wire_shape:?}",
                config.slug
            ),
        }),
    }
}

/// Writes the fields every body of the shape carries with the same value
/// (`shapeFixedValue`, owner decision 4): on ChatOpenAI the object type,
/// the choice index and the message role.
fn encode_fixed_values(raw: &mut Value, chat_wire_shape: &str) {
    for fixed in wire_shape_fixed_values(chat_wire_shape) {
        let value = match fixed.value {
            FixedJson::Str(text) => json!(text),
            FixedJson::Int(number) => json!(number),
        };
        set_wire_path(raw, fixed.path, value);
    }
}

/// Maps the provider's finish-reason string to its canonical value through the
/// chat wire shape's generated table (ADR-099 CRF-001). A string with no entry
/// is `FinishReason::Other` (CRF-011); no string at all is `None`. `raw` is read
/// only for an entry whose wire value fuses two canonical values (CRF-007); a
/// caller with no body passes `None`, and such an entry then does not match.
pub(crate) fn decode_finish_reason(
    chat_wire_shape: &str,
    wire: Option<&str>,
    raw: Option<&Value>,
) -> Option<FinishReason> {
    let wire = wire?;
    let entry = wire_shape_finish_reasons(chat_wire_shape).iter().find(|e| {
        e.wire_value == wire
            && (e.detail_path.is_empty()
                || raw.is_some_and(|raw| extract_string_path(raw, e.detail_path) == e.detail_value))
    });
    Some(entry.map_or(FinishReason::Other, |e| e.canonical))
}

/// Picks the wire value `encode_response` writes (ADR-099 CRF-010). The
/// provider's own string wins when the target shape knows it with the same
/// canonical value, so a same-format round trip is exact (Google RECITATION
/// stays RECITATION). A response decoded from the target shape also keeps a
/// string the table does not know (owner decision 1, HANDOFF-105 F3):
/// Anthropic pause_turn decodes as `Other` and encodes back as pause_turn.
/// Otherwise the preferred entry for the canonical value is
/// written (Anthropic end_turn becomes OpenAI stop). A canonical value the
/// shape has no entry for is written as the shape's declared fallback (owner
/// decision 1, HANDOFF-105 section 3b): ChatOpenAI `Other` is `stop`. An
/// unmapped provider string, such as Google LANGUAGE, is not a known wire
/// value, so it too encodes as the fallback (CRF-010).
///
/// `None` only when the response reports no finish reason, or the shape
/// declares no table: there is nothing to write.
fn encode_finish_reason<'a>(
    chat_wire_shape: &str,
    response: &'a Response,
) -> Option<FinishReasonWrite<'a>> {
    let canonical = response.finish_reason?;
    let table = wire_shape_finish_reasons(chat_wire_shape);
    if let Some(known) = table.iter().find(|e| {
        Some(e.wire_value) == response.provider_finish_reason.as_deref() && e.canonical == canonical
    }) {
        return Some(FinishReasonWrite::from(known));
    }
    // After the table, so a fused entry (CRF-007) keeps its detail.
    if response.chat_wire_shape.as_deref() == Some(chat_wire_shape) {
        if let Some(wire) = response.provider_finish_reason.as_deref() {
            return Some(FinishReasonWrite::bare(wire));
        }
    }
    table
        .iter()
        .find(|e| e.preferred && e.canonical == canonical)
        .map(FinishReasonWrite::from)
        .or_else(|| {
            wire_shape_finish_reason_fallback(chat_wire_shape, canonical)
                .map(FinishReasonWrite::bare)
        })
}

/// What `encode_response` writes for the finish reason: the wire value, and
/// the detail field a fused entry (CRF-007) also sets. The wire value borrows
/// from the response when it is the provider's own unmapped string.
struct FinishReasonWrite<'a> {
    wire_value: &'a str,
    detail_path: &'static str,
    detail_value: &'static str,
}

impl<'a> FinishReasonWrite<'a> {
    fn bare(wire_value: &'a str) -> Self {
        Self {
            wire_value,
            detail_path: "",
            detail_value: "",
        }
    }
}

impl From<&FinishReasonEntry> for FinishReasonWrite<'_> {
    fn from(e: &FinishReasonEntry) -> Self {
        Self {
            wire_value: e.wire_value,
            detail_path: e.detail_path,
            detail_value: e.detail_value,
        }
    }
}

/// [`decode_response`] plus the ADR-014 raw opt-in. Every Response send path
/// (prompt, agent, batch) decodes or attaches through here, so none can forget
/// the caller's `.raw()` (BUG-073). The public codec keeps its signature
/// (ADR-076).
pub(crate) fn decode_response_raw(
    provider: ProviderName,
    chat_wire_shape: &str,
    body: &str,
    raw: bool,
) -> Result<Response, Error> {
    let response = decode_response(provider, chat_wire_shape, body)?;
    Ok(attach_raw(response, body, raw))
}

/// Sets `response.raw` to the parsed `body` when the caller opted in.
pub(crate) fn attach_raw(mut response: Response, body: &str, raw: bool) -> Response {
    if raw {
        response.raw = serde_json::from_str(body).ok();
    }
    response
}

/// Reads every canonical [`Usage`] dimension out of a provider response body.
/// The ONE usage reader (ADR-076 SYM-004): the codec, the chat send path and
/// the agent loop all call this, so a dimension cannot be read in one place and
/// forgotten in another — which is precisely how the agent loop came to
/// accumulate three of six (BUG-045).
///
/// A dimension is `None` when the provider declares no path for it OR the
/// response did not carry it. Neither is zero.
pub(crate) fn decode_usage(raw: &Value, provider: ProviderName) -> Usage {
    let (input_path, output_path) = usage_paths(provider);
    let (write_path, read_path) = cache_usage_paths(provider);
    let cfg = provider_config(provider);
    Usage {
        input: opt_int_path(raw, input_path),
        output: opt_int_path(raw, output_path),
        cache_write: opt_int_path(raw, write_path),
        cache_read: opt_int_path(raw, read_path),
        reasoning: opt_int_path(raw, cfg.reasoning_tokens_path),
        cost: scale_cost(
            opt_f64_path(raw, usage_cost_path(provider)),
            usage_cost_scale(provider),
        ),
    }
}

/// Applies the per-provider cost scale, PRESERVING absence: an unreported cost
/// stays unreported rather than becoming `0.0 * scale` (AVAIL-007). A request
/// nobody priced is not a free request.
fn scale_cost(cost: Option<f64>, scale: f64) -> Option<f64> {
    cost.map(|value| value * scale)
}

/// Absorbing addition over one optional dimension (ADR-081 AVAIL-005): a total
/// is only reported when EVERY summand reported it. Summing the turns that
/// happened to report a dimension and presenting that as the total is the
/// original defect at aggregate scale.
fn add_opt<T: std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + b),
        _ => None,
    }
}

/// Adds two [`Usage`] values dimension-wise, absorbing per dimension. Used by
/// the agent tool loop and by batch aggregation.
pub(crate) fn accumulate_usage(total: Usage, turn: Usage) -> Usage {
    Usage {
        input: add_opt(total.input, turn.input),
        output: add_opt(total.output, turn.output),
        cache_write: add_opt(total.cache_write, turn.cache_write),
        cache_read: add_opt(total.cache_read, turn.cache_read),
        reasoning: add_opt(total.reasoning, turn.reasoning),
        cost: add_opt(total.cost, turn.cost),
    }
}

/// [`decode_response`]'s inverse: renders a canonical [`Response`] back onto the
/// wire for `provider` + `chat_wire_shape`. Every write location comes from the
/// same generated path accessors `decode_response` reads — there is no second
/// table and no path literal here (ADR-076 SYM-005).
///
/// Keyless, IO-free and pure, like its mirror. The result is NOT byte-identical
/// to the body a provider would send: a provider body carries fields the
/// canonical `Response` does not model (ADR-014's `raw` exists for exactly
/// that). The contract is the canonical fixed point,
/// `decode(encode(decode(b))) == decode(b)` (SYM-006).
pub fn encode_response(
    provider: ProviderName,
    chat_wire_shape: &str,
    response: &Response,
) -> Result<String, Error> {
    let chat_wire_shape = served_chat_wire_shape(provider, chat_wire_shape)?;
    guard_one_way_fields(provider, response)?;
    check_turn_parts("assistant", &response.parts)?;
    if chat_wire_shape == "ChatResponsesOpenAI" {
        return Ok(serde_json::to_string(&encode_responses_envelope(
            provider,
            chat_wire_shape,
            response,
        )?)?);
    }

    let mut raw = Value::Object(Map::new());
    encode_chat_parts(&mut raw, provider, chat_wire_shape, response)?;
    let (input_path, output_path) = usage_paths(provider);
    set_wire_path(&mut raw, input_path, json!(response.usage.input));
    set_wire_path(&mut raw, output_path, json!(response.usage.output));
    let (write_path, read_path) = cache_usage_paths(provider);
    set_wire_path(&mut raw, write_path, json!(response.usage.cache_write));
    set_wire_path(&mut raw, read_path, json!(response.usage.cache_read));
    let scale = usage_cost_scale(provider);
    if scale != 0.0 {
        // An unreported cost has no wire location to fill; `json!(None)` is
        // Null, which set_wire_path skips.
        let cost = json!(response.usage.cost.map(|value| value / scale));
        set_wire_path(&mut raw, usage_cost_path(provider), cost);
    }
    let cfg = provider_config(provider);
    set_wire_path(
        &mut raw,
        cfg.reasoning_tokens_path,
        json!(response.usage.reasoning),
    );
    if let Some(entry) = encode_finish_reason(chat_wire_shape, response) {
        set_wire_path(&mut raw, cfg.finish_reason_path, json!(entry.wire_value));
        set_wire_path(&mut raw, entry.detail_path, json!(entry.detail_value));
    }
    set_wire_path(
        &mut raw,
        cfg.finish_message_path,
        json!(response.finish_message),
    );
    set_wire_path(&mut raw, cfg.response_id_path, json!(response.id));
    set_wire_path(&mut raw, cfg.response_model_path, json!(response.model));
    set_wire_path(
        &mut raw,
        wire_shape_response_created_path(chat_wire_shape),
        json!(response.created),
    );
    encode_fixed_values(&mut raw, chat_wire_shape);
    Ok(serde_json::to_string(&raw)?)
}

/// Refuses to encode a canonical field whose mapping is `OneWay` for this
/// provider — the result set of CQ-WMAP-011 restricted to `Response`
/// (ADR-076 SYM-007). That set is empty today: `exceptGoogleToolCallID`
/// is on `ToolCall`, whose Google id the config never writes
/// (HANDOFF-105 F4), and
/// `exceptVertexFinishReason` is on the image response since ADR-099 slice
/// C4 (CRF-006). A Vertex chat `Response` writes no finish reason anyway: Vertex
/// has no chat wire shape, so `encode_finish_reason` finds no table.
///
/// The guard stays so a new whole-field exception on `Response` fails
/// lint_response_codec.py until it is guarded here. An empty value is not an
/// error: there is nothing to write. `field` and `message` carry the mapping's
/// `canonicalPath` and `invertibilityNote` verbatim.
fn guard_one_way_fields(_provider: ProviderName, _response: &Response) -> Result<(), Error> {
    Ok(())
}

/// Extracts text + usage from OpenAI's Responses reply (ADR-055). Unlike Chat
/// Completions (choices[].message.content), the reply is an `output[]` array
/// whose message item carries `content[]` blocks of type "output_text"; usage
/// is input_tokens/output_tokens with cached + reasoning sub-details.
/// Live-anchored 2026-07-02. Hand-coded per wire shape, symmetric with the
/// request-side `input` envelope (ADR-028: behavior held by tests, not by
/// declared response paths).
fn parse_responses_envelope(raw: &Value) -> Response {
    let text = extract_responses_text(raw);
    Response {
        parts: if text.is_empty() {
            Vec::new()
        } else {
            vec![Part::Text(text)]
        },
        usage: Usage {
            input: opt_int_path(raw, "usage.input_tokens"),
            output: opt_int_path(raw, "usage.output_tokens"),
            // The Responses envelope carries no cache-write or cost field at
            // all. That is not a zero: it is the provider never making the
            // claim (ADR-081).
            cache_write: None,
            cache_read: opt_int_path(raw, "usage.input_tokens_details.cached_tokens"),
            reasoning: opt_int_path(raw, "usage.output_tokens_details.reasoning_tokens"),
            cost: None,
        },
        finish_reason: decode_finish_reason(
            "ChatResponsesOpenAI",
            opt_string(extract_string_path(raw, "status")).as_deref(),
            Some(raw),
        ),
        provider_finish_reason: opt_string(extract_string_path(raw, "status")),
        finish_message: None,
        raw: None,
        // Set by the caller (decode_response), which holds the original text.
        // This shape's turn is an item LIST, not a message object.
        provider_turn: None,
        id: opt_string(extract_string_path(
            raw,
            wire_shape_response_id_path("ChatResponsesOpenAI"),
        )),
        model: opt_string(extract_string_path(
            raw,
            wire_shape_response_model_path("ChatResponsesOpenAI"),
        )),
        state: None,
        cause: None,
        created: opt_int_path(raw, wire_shape_response_created_path("ChatResponsesOpenAI")),
        // Set by the caller (decode_response), which holds the shape.
        chat_wire_shape: None,
    }
}

/// A parsed signal string as an optional canonical field: an empty extraction
/// means the provider declared no path or sent no value, which is absence, not
/// the empty string.
pub(crate) fn opt_string(value: String) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// Mirror of `parse_responses_envelope`: rebuilds OpenAI's Responses reply
/// (ADR-055) — an `output[]` array whose message item carries `content[]`
/// blocks of type "output_text", with input_tokens/output_tokens usage and
/// cached + reasoning sub-details. Hand-coded per wire shape on both legs,
/// symmetric with the reader, for the same reason the reader is.
///
/// The shape declares no `ToolCallResponseConfig`, so a response with tool
/// calls is `Error::Validation` on field `tool_calls` (HANDOFF-105 F4
/// decision 2).
fn encode_responses_envelope(
    provider: ProviderName,
    chat_wire_shape: &str,
    response: &Response,
) -> Result<Value, Error> {
    if !response.tool_calls().is_empty() {
        return Err(no_tool_call_config_error(provider, chat_wire_shape));
    }
    let mut raw = Value::Object(Map::new());
    let text = response.text();
    if !text.is_empty() {
        raw["output"] = json!([{
            "type": "message",
            "content": [{"type": "output_text", "text": text}],
        }]);
    }
    set_wire_path(&mut raw, "usage.input_tokens", json!(response.usage.input));
    set_wire_path(
        &mut raw,
        "usage.output_tokens",
        json!(response.usage.output),
    );
    set_wire_path(
        &mut raw,
        "usage.input_tokens_details.cached_tokens",
        json!(response.usage.cache_read),
    );
    set_wire_path(
        &mut raw,
        "usage.output_tokens_details.reasoning_tokens",
        json!(response.usage.reasoning),
    );
    if let Some(entry) = encode_finish_reason(chat_wire_shape, response) {
        set_wire_path(&mut raw, "status", json!(entry.wire_value));
        set_wire_path(&mut raw, entry.detail_path, json!(entry.detail_value));
    }
    set_wire_path(
        &mut raw,
        wire_shape_response_id_path("ChatResponsesOpenAI"),
        json!(response.id),
    );
    set_wire_path(
        &mut raw,
        wire_shape_response_model_path("ChatResponsesOpenAI"),
        json!(response.model),
    );
    set_wire_path(
        &mut raw,
        wire_shape_response_created_path("ChatResponsesOpenAI"),
        json!(response.created),
    );
    encode_fixed_values(&mut raw, chat_wire_shape);
    Ok(raw)
}

/// Walks the Responses `output[]` array for the first message item and returns
/// its first `output_text` block. Iterating (rather than a fixed
/// output[0].content[0] path) tolerates a leading reasoning item.
fn extract_responses_text(raw: &Value) -> String {
    let Some(output) = raw.get("output").and_then(Value::as_array) else {
        return String::new();
    };
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(content) = item.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("output_text") {
                continue;
            }
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                return text.to_string();
            }
        }
    }
    String::new()
}

/// Pull the provider stop signal + free-text explanation from a response
///
/// empty strings when the provider declares no path or the value is not
/// present in this response.
pub(crate) fn extract_finish_signal(
    raw: &Value,
    provider: ProviderName,
) -> (Option<String>, Option<String>) {
    let cfg = provider_config(provider);
    (
        opt_string(extract_string_path(raw, cfg.finish_reason_path)),
        opt_string(extract_string_path(raw, cfg.finish_message_path)),
    )
}

pub fn parse_api_error(provider: &Provider, status_code: u16, body: &str) -> Error {
    let config = provider_config(provider.name);
    let parsed: Result<Value, _> = serde_json::from_str(body);
    let message = parsed
        .ok()
        .and_then(|raw| {
            if config.error_message_path.is_empty() {
                None
            } else {
                Some(extract_string_path(&raw, config.error_message_path))
            }
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| body.to_string());

    Error::Api {
        provider: config.slug.to_string(),
        status_code,
        message,
        reason: api_error_reason(provider.name, body),
    }
}

/// The provider's machine-readable error code in an error body, read at the
///
/// declares no path, the body is not JSON, or the body carries no reason.
pub(crate) fn api_error_reason(provider: ProviderName, body: &str) -> Option<String> {
    let path = provider_config(provider).error_reason_path;
    if path.is_empty() {
        return None;
    }
    let raw: Value = serde_json::from_str(body).ok()?;
    opt_string(extract_string_path(&raw, path))
}





































