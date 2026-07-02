// ADR-054 opt-in telemetry — cross-SDK OTLP parity + runtime wiring.
//
// The parity tests call the PURE builder `build_otlp_traces` with the fixed
// TEL-011 inputs and assert the payload is JSON-value-equal to the shared
// golden every SDK asserts against (codegen/testdata/wire/telemetry/v1/*.json).
// Artifacts are dropped at target/wire/telemetry/<fixture>/rust.json for the
// cross-SDK comparator, mirroring the request-wire suite (tests/request_wire.rs).
//
// The mock-collector test drives the FULL chat path: a client with telemetry
// attached prompts a mock LLM, and the synchronous post-phase exporter POSTs an
// OTLP span to a std::net collector — validating the middleware wiring end to
// end. The empty-endpoint test pins the construction-time fail-loud contract.

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use common::{serve_once, TestResponse};
use llmkit::builders::openai;
use llmkit::{build_otlp_traces, Telemetry};

// Reads a full HTTP/1.1 request (headers + Content-Length body). A single
// read() can return only the header segment, so loop until the body arrives.
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

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

// Writes the SDK artifact for the cross-SDK comparator and asserts the payload
// parses JSON-value-equal to the shared golden.
fn assert_telemetry_wire_golden(fixture: &str, payload: &str) {
    let root = repo_root();
    let artifact = root.join(format!("target/wire/telemetry/{fixture}/rust.json"));
    std::fs::create_dir_all(artifact.parent().unwrap()).expect("mkdir artifact dir");
    std::fs::write(&artifact, payload).expect("write artifact");

    let golden_path = root.join(format!("codegen/testdata/wire/telemetry/v1/{fixture}.json"));
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("read golden"))
            .expect("parse golden");
    let actual: serde_json::Value = serde_json::from_str(payload).expect("parse rust payload");
    assert_eq!(
        actual, golden,
        "Rust {fixture} OTLP payload differs from shared golden"
    );
}

#[test]
fn telemetry_wire_success_golden() {
    let payload = build_otlp_traces(
        "chat",
        "openai",
        "gpt-4o",
        10,
        20,
        "",
        "5b8efff798038103d269b633813fc60c",
        "eee19b7ec3c1b174",
        "1700000000000000000",
        "1700000001000000000",
    );
    assert_telemetry_wire_golden("telemetry-success", &payload);
}

#[test]
fn telemetry_wire_rejection_golden() {
    let payload = build_otlp_traces(
        "chat",
        "openai",
        "gpt-4o",
        0,
        0,
        "rate_limit_exceeded",
        "5b8efff798038103d269b633813fc60c",
        "eee19b7ec3c1b174",
        "1700000000000000000",
        "1700000001000000000",
    );
    assert_telemetry_wire_golden("telemetry-rejection", &payload);
}

// End-to-end: telemetry attached to a client exports an OTLP span over the
// middleware seam on a real chat call. Two mock servers — one for the LLM
// (serve_once), one std::net collector — with a synchronous export so the
// assertion is deterministic after the prompt resolves.
#[tokio::test]
async fn telemetry_exports_over_chat_path() {
    // The OTLP collector: accept one connection, capture the request, reply 200.
    let collector = TcpListener::bind("127.0.0.1:0").expect("bind collector");
    let collector_addr = collector.local_addr().expect("collector addr");
    let (tx, rx) = mpsc::channel::<String>();
    let collector_thread = thread::spawn(move || {
        let (mut stream, _) = collector.accept().expect("accept");
        let request = read_full_http_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .expect("respond");
        tx.send(request).expect("send captured request");
    });

    // The mock LLM: a valid OpenAI chat response so the prompt succeeds and the
    // post phase fires with usage.
    let llm_url = serve_once(
        |_request, _json| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "Hello!"}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 20}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut headers = std::collections::HashMap::new();
    headers.insert("authorization".to_string(), "Bearer collector-secret".to_string());
    let tel = Telemetry {
        endpoint: format!("http://{collector_addr}"),
        headers,
        capture_content: false,
    };

    let mut client = openai("test-key").with_telemetry(tel);
    client.provider.base_url = Some(llm_url);
    let response = client
        .text()
        .model("gpt-4o")
        .prompt("Hi")
        .await
        .expect("prompt succeeds");
    assert_eq!(response.usage.output, 20);

    collector_thread.join().expect("collector thread");
    let request = rx.recv().expect("captured export request");
    let request_lower = request.to_lowercase();
    assert!(
        request_lower.starts_with("post /v1/traces http/1.1"),
        "telemetry must POST to /v1/traces, got: {}",
        request.lines().next().unwrap_or("")
    );
    assert!(
        request_lower.contains("authorization: bearer collector-secret\r\n"),
        "caller export header must ride the OTLP POST"
    );
    assert!(
        request.contains("\"resourceSpans\""),
        "export body must carry the OTLP resourceSpans payload"
    );
    assert!(
        request.contains("\"gen_ai.operation.name\""),
        "export body must carry the gen_ai operation attribute"
    );
}

// The honest-contract lineage (ADR-054): telemetry with no sink is a
// construction-time programmer error, not a silent no-op.
#[test]
#[should_panic(expected = "telemetry.endpoint")]
fn with_telemetry_empty_endpoint_panics() {
    let _ = openai("test-key").with_telemetry(Telemetry {
        endpoint: String::new(),
        headers: std::collections::HashMap::new(),
        capture_content: false,
    });
}
