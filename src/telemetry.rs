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
//!
//! into `providers::generated::telemetry`; this handwritten layer only
//! carries runtime behaviour (config, span identity, OTLP encoding, the
//! optional `http_export` transport). A handwritten config value like the
//!
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
        //
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

/// Classifies the post-phase `Event` and renders it to the OTLP traces JSON.
/// Span identity + timing are stamped here (the pure builder takes them as
/// arguments so the parity goldens can inject fixed values).
fn build_telemetry_payload(e: &Event) -> String {
    let op = telemetry_operation_name(e.op)
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{:?}", e.op));
    let (input, output) = e.usage.map(|u| (u.input, u.output)).unwrap_or((0, 0));
    let error_type = e.err.as_deref().map(classify_error).unwrap_or_default();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());

    build_otlp_traces(
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



































































































































































