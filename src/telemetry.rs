//! Opt-in, OTEL GenAI-aligned telemetry (ADR-059, superseding ADR-054's
//! transport half).
//!
//! Mirrors the Go reference (`go/telemetry.go`). Attach a [`Telemetry`]
//! config with [`Client::add_telemetry`]: on every capability path that
//! fires middleware — success and rejection alike — llmkit builds an OTEL
//! GenAI-aligned OTLP span (proto3 JSON) and hands the finished bytes to the
//! `export` callback. llmkit does no telemetry network I/O and spawns no
//! thread; batching/backpressure/shutdown is the caller's concern. Use
//! [`http_export`] for a batteries POST.
//!
//! The OTEL GenAI binding facts (semconv version, attribute keys, the
//! `MiddlewareOp` -> operation-name map) are generated from the ontology
//! into `providers::generated::telemetry`; this handwritten layer only
//! carries runtime behaviour (config, span identity, OTLP encoding, the
//! optional `http_export` transport). A handwritten config value like the
//! ADR-052 baseURL / custom-header overrides — not modelled in the ontology.
//!
//! Divergences from Go, forced by Rust's shape:
//! - The honest contract (TEL-017) is enforced by the type system, not a
//!   runtime check: `export` is a required, non-null field, so an
//!   enabled-but-no-sink `Telemetry` is unrepresentable (Go/TS/Python guard a
//!   nullable callback at runtime).
//! - `http_export` is a synchronous `std::net` HTTP/1.1 client (http only) run
//!   inline on the post phase — no thread. A slow collector adds latency to the
//!   batteries caller (documented low-volume); the BYO callback owns its own
//!   dispatch. Every export error is swallowed (fail-open).
//! - The middleware `Event.err` is a `String` (the typed error is lost at
//!   the seam), so `error.type` is read verbatim from `Event.err_type`,
//!   stamped structurally at the erasure seam (`set_event_error`, ADR-071).

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
    telemetry_operation_name, OTEL_ATTR_ERR_TYPE, OTEL_ATTR_MODEL, OTEL_ATTR_OP, OTEL_ATTR_PROVIDER,
    OTEL_USAGE_INPUT, OTEL_USAGE_OUTPUT, TELEMETRY_SEMCONV_VERSION, TELEMETRY_TRACES_PATH,
};

/// The telemetry export callback: receives the finished OTLP/HTTP proto3-JSON
/// bytes for one span, called synchronously on the post phase. Mandatory and
/// non-null on [`Telemetry`], so an enabled-but-no-sink config is
/// unrepresentable (the honest-contract lineage, ADR-059 TEL-017).
pub type TelemetryExport = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Opt-in observability config (ADR-059). Attach with
/// [`Client::add_telemetry`]: llmkit builds an OTEL GenAI-aligned OTLP span on
/// every provider call and hands the finished bytes to `export`. Off unless
/// attached; `export` is a required, non-null field so an enabled-but-no-sink
/// config cannot be constructed.
#[derive(Clone)]
pub struct Telemetry {
    /// Receives the finished OTLP bytes for one span, called synchronously on
    /// the post phase (mandatory). Use [`http_export`] for the batteries POST,
    /// or supply your own to bridge into an existing OTEL stack.
    pub export: TelemetryExport,
    /// Gates tier-2 message payloads (default `false` for privacy). The
    /// middleware `Event` does not carry payloads yet, so this reserves the
    /// semantics; content-log emission is a deferred follow-up (ADR-054 tier 2).
    pub capture_content: bool,
}

impl Client {
    /// Enable opt-in telemetry on this client. The builder rides the middleware
    /// seam, so every capability builder that carries a middleware seam
    /// (text/agent/image/music/video/upload) emits one OTEL span on the post
    /// phase. Chainable (`Client::new(...).add_telemetry(...)`).
    ///
    /// The honest contract (TEL-017) is upheld by the type system: `t.export`
    /// is a required, non-null field, so an enabled-but-no-sink `Telemetry`
    /// cannot be constructed — no runtime panic is needed.
    pub fn add_telemetry(mut self, t: Telemetry) -> Self {
        // Seed the export hook into the client's generic default middleware;
        // each on-demand builder clones it at construction (codegen owns the
        // seam, telemetry owns the hook).
        self.default_middleware
            .push(make_telemetry_middleware(t));
        self
    }
}

/// Builds the export hook. The post phase builds the OTLP payload and calls
/// `export` SYNCHRONOUSLY (ADR-059) — no thread. Fail-open: a panicking callback
/// is caught (`catch_unwind`) so telemetry never surfaces to the caller, parity
/// with the Go recover / TS try / Python except. Pre phase is a no-op.
fn make_telemetry_middleware(t: Telemetry) -> MiddlewareFn {
    Arc::new(move |e: &Event| {
        if e.phase == MiddlewarePhase::Post {
            let payload = build_telemetry_payload(e);
            let export = t.export.clone();
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                export(payload.as_bytes());
            }));
        }
        None
    })
}

/// Production wrapper: stamp span identity + timing, then render the `Event`.
fn build_telemetry_payload(e: &Event) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());
    build_telemetry_payload_at(e, &rand_hex(16), &rand_hex(8), &now, &now)
}

/// Pure event-level payload builder: renders a post-phase `Event` to the OTLP
/// traces JSON with injected span identity + timing (the telemetry-error
/// golden drives it end-to-end). `error.type` is `e.err_type` verbatim —
/// stamped structurally at the erasure seam (ADR-071), never re-derived here
/// from the message string.
pub fn build_telemetry_payload_at(
    e: &Event,
    trace_id: &str,
    span_id: &str,
    start_nano: &str,
    end_nano: &str,
) -> String {
    let op = telemetry_operation_name(e.op)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{:?}", e.op));
    let (input, output) = e.usage.map(|u| (u.input, u.output)).unwrap_or((0, 0));

    build_otlp_traces(
        &op,
        &e.provider,
        &e.model,
        input,
        output,
        &e.err_type,
        trace_id,
        span_id,
        start_nano,
        end_nano,
    )
}

/// Returns an [`TelemetryExport`] callback that POSTs each OTLP payload to
/// `endpoint` + `"/v1/traces"` with the given headers, fail-open (every error
/// is swallowed). It spawns no background worker and needs no shutdown.
///
/// Low-volume only: the POST is SYNCHRONOUS on the request path (a small
/// `std::net` HTTP/1.1 client, http only), so a slow or hung collector adds up
/// to ~5s of latency to the call. For high volume, hand your own `export`
/// callback that enqueues into your OTEL SDK's batch processor instead.
pub fn http_export(endpoint: &str, headers: HashMap<String, String>) -> TelemetryExport {
    let url = format!("{}{}", endpoint.trim_end_matches('/'), TELEMETRY_TRACES_PATH);
    Arc::new(move |payload: &[u8]| {
        let mut hdrs: Vec<(String, String)> =
            vec![("content-type".to_string(), "application/json".to_string())];
        for (k, v) in &headers {
            hdrs.push((k.clone(), v.clone()));
        }
        let _ = http_post_sync(&url, payload, &hdrs);
    })
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
            "key": OTEL_ATTR_ERR_TYPE,
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

    fn post_event() -> Event {
        Event {
            op: MiddlewareOp::LlmRequest,
            provider: "openai".to_string(),
            model: "gpt-4o".to_string(),
            phase: MiddlewarePhase::Post,
            usage: Some(crate::middleware::Usage {
                input: 10,
                output: 20,
                ..crate::middleware::Usage::default()
            }),
            ..Event::default()
        }
    }

    // ADR-059: the post phase hands finished OTLP bytes to the callback exactly
    // once, synchronously (populated by the time mw returns — no thread). The
    // pre phase never exports.
    #[test]
    fn export_callback_invoked_synchronously() {
        let captured: Arc<std::sync::Mutex<Vec<Vec<u8>>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        let tel = Telemetry {
            export: Arc::new(move |b: &[u8]| sink.lock().unwrap().push(b.to_vec())),
            capture_content: false,
        };
        let mw = make_telemetry_middleware(tel);

        let pre = Event {
            phase: MiddlewarePhase::Pre,
            ..post_event()
        };
        assert!(mw(&pre).is_none(), "pre phase must not veto");
        assert_eq!(captured.lock().unwrap().len(), 0, "pre phase must not export");

        assert!(mw(&post_event()).is_none(), "post export must be fail-open");
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1, "post phase must export exactly once");
        let body = String::from_utf8(got[0].clone()).expect("utf8 payload");
        assert!(
            body.contains("\"resourceSpans\""),
            "export payload must carry the OTLP resourceSpans envelope"
        );
    }

    // Fail-open: a panicking caller callback never surfaces on the request path.
    #[test]
    fn export_panicking_callback_fails_open() {
        let tel = Telemetry {
            export: Arc::new(|_b: &[u8]| panic!("callback blew up")),
            capture_content: false,
        };
        let mw = make_telemetry_middleware(tel);
        assert!(
            mw(&post_event()).is_none(),
            "a panicking callback must fail open"
        );
    }

    // Drives the batteries http_export through the middleware against a std::net
    // mock collector and asserts the OTLP POST lands at /v1/traces carrying the
    // resourceSpans payload + a caller header. The POST is synchronous, so the
    // collector has the request by the time mw returns.
    #[test]
    fn http_export_posts_otlp_to_mock_collector() {
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
            export: http_export(&format!("http://{addr}"), headers),
            capture_content: false,
        };
        let mw = make_telemetry_middleware(tel);

        let veto = mw(&post_event());
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
    fn http_export_fails_open_on_dead_endpoint() {
        let tel = Telemetry {
            export: http_export("http://127.0.0.1:1", HashMap::new()),
            capture_content: false,
        };
        let mw = make_telemetry_middleware(tel);
        assert!(mw(&post_event()).is_none(), "a dead endpoint must fail open");
    }
}
