//
//
//
//
//
//
//
//
//
//
//
//

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use std::sync::{Arc, Mutex};

use common::{serve_once, TestResponse};
use llmkit::builders::openai;
use llmkit::telemetry::build_telemetry_payload_at;
use llmkit::{
    build_otlp_traces, http_export, set_event_error, Error, Event, MiddlewareOp,
    MiddlewarePhase, Telemetry,
};

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

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

//
//
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

#
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

#
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

//
//
//
//
#
fn telemetry_wire_error_golden() {
    let mut ev = Event {
        op: MiddlewareOp::LlmRequest,
        phase: MiddlewarePhase::Post,
        provider: "openai".to_string(),
        model: "gpt-4o".to_string(),
        ..Event::default()
    };
    set_event_error(
        &mut ev,
        &Error::Api {
            provider: "openai".to_string(),
            status_code: 429,
            message: "rate limited".to_string(),
        },
    );
    let payload = build_telemetry_payload_at(
        &ev,
        "5b8efff798038103d269b633813fc60c",
        "eee19b7ec3c1b174",
        "1700000000000000000",
        "1700000001000000000",
    );
    assert_telemetry_wire_golden("telemetry-error", &payload);
}

//
//
//
//
#
async fn telemetry_exports_over_chat_path() {
    //
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

    //
    //
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
        export: http_export(&format!("http://{collector_addr}"), headers),
        capture_content: false,
    };

    let mut client = openai("test-key").add_telemetry(tel);
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

//
//
//
#
async fn telemetry_byo_callback_receives_bytes_on_chat_path() {
    let captured: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    let tel = Telemetry {
        export: Arc::new(move |b: &[u8]| sink.lock().unwrap().push(b.to_vec())),
        capture_content: false,
    };

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

    let mut client = openai("test-key").add_telemetry(tel);
    client.provider.base_url = Some(llm_url);
    client
        .text()
        .model("gpt-4o")
        .prompt("Hi")
        .await
        .expect("prompt succeeds");

    let got = captured.lock().unwrap();
    assert_eq!(got.len(), 1, "the callback must receive one span per call");
    let body = String::from_utf8(got[0].clone()).expect("utf8 payload");
    assert!(
        body.contains("\"gen_ai.operation.name\""),
        "the callback must receive the OTLP span bytes"
    );
}

//
//
//
//
//
#
fn add_telemetry_requires_export_by_type() {
    let _ = openai("test-key").add_telemetry(Telemetry {
        export: Arc::new(|_b: &[u8]| {}),
        capture_content: false,
    });
}
