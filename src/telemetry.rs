//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

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

///
///
///
///
pub type TelemetryExport = Arc<dyn Fn(&[u8]) + Send + Sync>;

///
///
///
///
///
#
pub struct Telemetry {
    ///
    ///
    ///
    pub export: TelemetryExport,
    ///
    ///
    ///
    pub capture_content: bool,
}

impl Client {
    ///
    ///
    ///
    ///
    ///
    ///
    ///
    ///
    pub fn add_telemetry(mut self, t: Telemetry) -> Self {
        //
        //
        //
        self.default_middleware
            .push(make_telemetry_middleware(t));
        self
    }
}

///
///
///
///
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

///
fn build_telemetry_payload(e: &Event) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());
    build_telemetry_payload_at(e, &rand_hex(16), &rand_hex(8), &now, &now)
}

///
///
///
///
///
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

///
///
///
///
///
///
///
///
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

///
///
///
///
///
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

///
///
///
///
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

    //
    let mut sink = Vec::new();
    let _ = stream.read_to_end(&mut sink);
    Ok(())
}

///
///
///
///
///
///
///
///
///
#
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

#
mod tests {
    use super::*;
    use crate::middleware::MiddlewareOp;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    //
    //
    //
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

    //
    //
    //
    #
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

    //
    #
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

    //
    //
    //
    //
    #
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

    //
    #
    fn http_export_fails_open_on_dead_endpoint() {
        let tel = Telemetry {
            export: http_export("http://127.0.0.1:1", HashMap::new()),
            capture_content: false,
        };
        let mw = make_telemetry_middleware(tel);
        assert!(mw(&post_event()).is_none(), "a dead endpoint must fail open");
    }
}
