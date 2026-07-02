//! Opt-in, OTEL GenAI-aligned telemetry over OTLP/HTTP (ADR-054).
//!
//! Mirrors the Go reference (`go/telemetry.go`). Attach a [`Telemetry`]
//! config with [`Client::with_telemetry`]; the exporter rides the
//! middleware seam so every capability path that fires middleware emits
//! one OTEL span on the post phase — success and rejection alike.
//!
//! The OTEL GenAI binding facts (semconv version, attribute keys, the
//! `MiddlewareOp` -> operation-name map) are generated from the ontology
//! into `providers::generated::telemetry`; this handwritten layer only
//! carries runtime behaviour (config, span identity, OTLP encoding, the
//! fail-open export). A handwritten config value like the ADR-052
//! baseURL / custom-header overrides — not modelled in the ontology.
//!
//! Divergences from Go, forced by Rust's shape (see the handoff notes):
//! - Empty endpoint is a construction-time `panic` (the programmer-error
//!   idiom), not a deferred pre-phase veto — Go defers construction-time
//!   validation to first use; Rust fails loud at `with_telemetry`.
//! - The middleware seam is a *synchronous* closure, so the export POST is
//!   a small synchronous `std::net` HTTP/1.1 client (http only). A future
//!   async seam could route through the crate's `reqwest` helper and gain
//!   TLS. Every export error is swallowed (fail-open).
//! - The middleware `Event.err` is a `String` (the typed error is lost at
//!   the seam), so `error.type` is classified by message prefix.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::builders::Client;
use crate::middleware::{Event, MiddlewareFn, MiddlewarePhase};
use crate::providers::generated::telemetry::{
    telemetry_operation_name, OTEL_ATTR_ERR, OTEL_ATTR_MODEL, OTEL_ATTR_OP, OTEL_ATTR_PROVIDER,
    OTEL_USAGE_INPUT, OTEL_USAGE_OUTPUT, TELEMETRY_SEMCONV_VERSION, TELEMETRY_TRACES_PATH,
};

/// Opt-in observability config (ADR-054). Attach with
/// [`Client::with_telemetry`] to export an OTEL GenAI-aligned span over
/// OTLP/HTTP (JSON) on every provider call. Off unless attached; an empty
/// `endpoint` is a construction-time panic (the honest-contract lineage —
/// no enabled-but-no-sink state).
#[derive(Clone, Debug, Default)]
pub struct Telemetry {
    /// OTLP/HTTP collector base URL (mandatory). The exporter POSTs
    /// proto3-JSON to `endpoint` + `"/v1/traces"`.
    pub endpoint: String,
    /// Headers added to every export POST (e.g. `authorization`).
    pub headers: HashMap<String, String>,
    /// Gates tier-2 message payloads (default `false` for privacy). The
    /// middleware `Event` does not carry payloads yet, so this reserves the
    /// semantics; content-log emission is a deferred follow-up (ADR-054 tier 2).
    pub capture_content: bool,
}

impl Client {
    /// Enable opt-in telemetry on this client. The exporter rides the
    /// middleware seam, so every capability builder that carries a middleware
    /// seam (text/agent/image/music/video/upload) emits one OTEL span on the
    /// post phase. Chainable per the ADR-054 sketch
    /// (`Client::new(...).with_telemetry(...)`).
    ///
    /// # Panics
    /// Panics if `t.endpoint` is empty — an enabled-but-no-sink telemetry
    /// config is a programmer error, caught at construction rather than
    /// silently dropping spans.
    pub fn with_telemetry(mut self, t: Telemetry) -> Self {
        assert!(
            !t.endpoint.is_empty(),
            "telemetry.endpoint is required when telemetry is enabled"
        );
        // Seed the fail-open exporter into the client's generic default
        // middleware; each on-demand builder clones it at construction
        // (codegen owns the seam, telemetry owns the hook — ADR-054).
        self.default_middleware
            .push(make_telemetry_middleware(t));
        self
    }
}

/// Builds the export hook. The post phase exports fail-open: a telemetry
/// failure never propagates or blocks the call. Pre phase is a no-op (the
/// empty-endpoint contract is enforced at construction, so the closure only
/// ever sees a non-empty endpoint).
fn make_telemetry_middleware(t: Telemetry) -> MiddlewareFn {
    Arc::new(move |e: &Event| {
        if e.phase == MiddlewarePhase::Post {
            export_telemetry(&t, e);
        }
        None
    })
}

/// Serializes the post-phase `Event` to an OTLP traces payload and POSTs it.
/// Fail-open: every error (bad endpoint, timeout) is swallowed.
fn export_telemetry(t: &Telemetry, e: &Event) {
    let op = telemetry_operation_name(e.op)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{:?}", e.op));
    let (input, output) = e.usage.map(|u| (u.input, u.output)).unwrap_or((0, 0));
    let error_type = e.err.as_deref().map(classify_error).unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());

    let payload = build_otlp_traces(
        &op,
        &e.provider,
        &e.model,
        input,
        output,
        &error_type,
        &rand_hex(16),
        &rand_hex(8),
        &now,
        &now,
    );

    let url = format!("{}{}", t.endpoint.trim_end_matches('/'), TELEMETRY_TRACES_PATH);
    let mut headers: Vec<(String, String)> =
        vec![("content-type".to_string(), "application/json".to_string())];
    for (k, v) in &t.headers {
        headers.push((k.clone(), v.clone()));
    }
    let _ = http_post_sync(&url, payload.as_bytes(), &headers);
}

/// Maps a lossy `Event.err` message to a stable OTEL `error.type` value. The
/// typed error is erased at the middleware seam (`Event.err: Option<String>`),
/// so classification keys off the `Error` `Display` prefixes.
fn classify_error(err: &str) -> String {
    if err.is_empty() {
        return String::new();
    }
    if err.starts_with("validation:") {
        "validation_error".to_string()
    } else if err.starts_with("http:")
        || err.starts_with("json:")
        || err.starts_with("unsupported:")
        || err.starts_with("middleware veto:")
    {
        "error".to_string()
    } else {
        // `Error::Api` renders as "{provider}: {message} ({status})".
        "api_error".to_string()
    }
}

/// A non-crypto, unique-per-call hex string of `n_bytes` bytes for span/trace
/// identity. The zero-CSPRNG-dependency posture mirrors `new_video_trace_id`:
/// uniqueness is sourced from the nanosecond clock mixed with a process-global
/// atomic counter, spread across the bytes via an xorshift. Collectors treat
/// these as opaque ids, so unpredictability is not required.
fn rand_hex(n_bytes: usize) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut state = nanos ^ count.rotate_left(32).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut bytes = Vec::with_capacity(n_bytes);
    for _ in 0..n_bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.push((state & 0xff) as u8);
    }
    hex::encode(bytes)
}

/// Minimal synchronous HTTP/1.1 POST over `std::net` (http only). Used because
/// the middleware seam is a synchronous closure; the crate's `reqwest` helper
/// is async and cannot be awaited here. Errors surface as `io::Error` and are
/// swallowed by the caller (fail-open).
fn http_post_sync(url: &str, body: &[u8], headers: &[(String, String)]) -> std::io::Result<()> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "telemetry sync exporter supports http:// only",
        )
    })?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rfind(':') {
        Some(i) => (&authority[..i], authority[i + 1..].parse::<u16>().unwrap_or(80)),
        None => (authority, 80u16),
    };

    let mut stream = TcpStream::connect((host, port))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");

    stream.write_all(request.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()?;

    // Drain the response so the collector's write completes; result ignored.
    let mut sink = Vec::new();
    let _ = stream.read_to_end(&mut sink);
    Ok(())
}

/// The PURE, deterministic OTLP-payload builder (OTLP/HTTP, proto3-JSON).
/// Given the call's primitives plus injectable span identity + timing, returns
/// the exact JSON the exporter POSTs. The parity fixtures call it with fixed
/// inputs so all four SDKs are asserted value-identical (TEL-011).
///
/// Encoding notes (OTLP/JSON spec): int64 fields (times, token counts) render
/// as *strings*; `traceId`/`spanId` are hex; each attribute `value` object
/// carries exactly one of `stringValue` (XOR) `intValue`; the span `status`
/// key is present only on error (`code: 2`), omitted on success.
#[allow(clippy::too_many_arguments)]
pub fn build_otlp_traces(
    operation_name: &str,
    provider: &str,
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    error_type: &str,
    trace_id: &str,
    span_id: &str,
    start_nano: &str,
    end_nano: &str,
) -> String {
    let mut attributes = vec![
        json!({ "key": OTEL_ATTR_OP, "value": { "stringValue": operation_name } }),
        json!({ "key": OTEL_ATTR_PROVIDER, "value": { "stringValue": provider } }),
        json!({ "key": OTEL_ATTR_MODEL, "value": { "stringValue": model } }),
    ];
    if input_tokens > 0 {
        attributes.push(json!({
            "key": OTEL_USAGE_INPUT,
            "value": { "intValue": input_tokens.to_string() }
        }));
    }
    if output_tokens > 0 {
        attributes.push(json!({
            "key": OTEL_USAGE_OUTPUT,
            "value": { "intValue": output_tokens.to_string() }
        }));
    }
    if !error_type.is_empty() {
        attributes.push(json!({
            "key": OTEL_ATTR_ERR,
            "value": { "stringValue": error_type }
        }));
    }

    let mut span = json!({
        "traceId": trace_id,
        "spanId": span_id,
        "name": format!("{} {}", operation_name, model),
        "kind": 3,
        "startTimeUnixNano": start_nano,
        "endTimeUnixNano": end_nano,
        "attributes": attributes,
    });
    if !error_type.is_empty() {
        span["status"] = json!({ "code": 2 });
    }

    let payload = json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [
                    { "key": "service.name", "value": { "stringValue": "llmkit" } }
                ]
            },
            "scopeSpans": [{
                "scope": { "name": "llmkit", "version": TELEMETRY_SEMCONV_VERSION },
                "spans": [span]
            }]
        }]
    });
    payload.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::MiddlewareOp;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    // Reads a full HTTP/1.1 request (headers + Content-Length body) from a
    // stream. A single read() can return only the header segment (TCP
    // segmentation), so loop until the declared body has arrived.
    fn read_full_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut data = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let n = stream.read(&mut chunk).expect("read");
            if n == 0 {
                break;
            }
            data.extend_from_slice(&chunk[..n]);
            if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                let header_text = String::from_utf8_lossy(&data[..pos]);
                let content_length = header_text
                    .lines()
                    .find_map(|line| {
                        let lower = line.to_ascii_lowercase();
                        lower
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if data.len() - (pos + 4) >= content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&data).to_string()
    }

    // Drives the export middleware through a synthetic post-phase Event against
    // a std::net mock collector and asserts the OTLP POST lands at /v1/traces
    // carrying the resourceSpans payload + a caller header. Mirrors Go's
    // TestTelemetry_ExportsToMockCollector (which calls the middleware factory
    // directly rather than a full client round-trip). Kept as a #[cfg(test)]
    // unit test so the exporter internals need not widen the public surface.
    #[test]
    fn exporter_posts_otlp_to_mock_collector() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind collector");
        let addr = listener.local_addr().expect("addr");
        let (tx, rx) = mpsc::channel::<String>();

        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_full_http_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .expect("respond");
            tx.send(request).expect("send captured request");
        });

        let mut headers = HashMap::new();
        headers.insert("authorization".to_string(), "Bearer secret".to_string());
        let tel = Telemetry {
            endpoint: format!("http://{addr}"),
            headers,
            capture_content: false,
        };
        let mw = make_telemetry_middleware(tel);

        let mut event = Event {
            op: MiddlewareOp::LlmRequest,
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            phase: MiddlewarePhase::Post,
            ..Event::default()
        };
        event.usage = Some(crate::middleware::Usage {
            input: 10,
            output: 20,
            ..crate::middleware::Usage::default()
        });
        let veto = mw(&event);
        assert!(veto.is_none(), "post-phase export must not veto");

        handle.join().expect("collector thread");
        let request = rx.recv().expect("captured request");
        let request_lower = request.to_lowercase();
        assert!(
            request_lower.starts_with("post /v1/traces http/1.1"),
            "exporter must POST to /v1/traces, got: {}",
            request.lines().next().unwrap_or("")
        );
        assert!(
            request_lower.contains("authorization: bearer secret\r\n"),
            "caller header must ride the export POST"
        );
        assert!(
            request.contains("\"resourceSpans\""),
            "export body must carry the OTLP resourceSpans payload"
        );
        assert!(
            request.contains("\"gen_ai.usage.input_tokens\""),
            "export body must carry usage attributes from the Event"
        );
    }

    // Fail-open: an unreachable endpoint never panics or surfaces.
    #[test]
    fn exporter_fails_open_on_dead_endpoint() {
        let tel = Telemetry {
            endpoint: "http://127.0.0.1:1".to_string(),
            headers: HashMap::new(),
            capture_content: false,
        };
        let mw = make_telemetry_middleware(tel);
        let event = Event {
            op: MiddlewareOp::LlmRequest,
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            phase: MiddlewarePhase::Post,
            ..Event::default()
        };
        assert!(mw(&event).is_none(), "a dead endpoint must fail open");
    }
}
