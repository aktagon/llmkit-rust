//! Smoke runner for `rust/examples/*.rs`.
//!
//! Cargo examples are separate crates, so this file can't `use` the
//! example modules directly. Each test re-implements the canonical
//! chain shown in the matching example file against a mock HTTP
//! server. Keep them in sync — if you change the chain in
//! `examples/<name>.rs`, update the mirror here too.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use base64::Engine;
use llmkit::builders::{anthropic, google, openai};
use llmkit::Tool;
use serde_json::Value;

// --- Mock server shared by all tests ---

struct TestResponse {
    status_line: &'static str,
    body: String,
    headers: Vec<(&'static str, &'static str)>,
}

struct TestExchange {
    assert_request: Box<dyn Fn(String, Value) + Send + 'static>,
    response: TestResponse,
}

fn serve_sequence(exchanges: Vec<TestExchange>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    thread::spawn(move || {
        for exchange in exchanges {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_http_request(&mut stream);
            let split = request
                .find("\r\n\r\n")
                .expect("http request separator present");
            let body_text = request[split + 4..].to_string();
            let json: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);
            (exchange.assert_request)(request, json);

            let mut response_text = format!(
                "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
                exchange.response.status_line,
                exchange.response.body.len()
            );
            for (name, value) in exchange.response.headers {
                response_text.push_str(&format!("{name}: {value}\r\n"));
            }
            response_text.push_str("\r\n");
            response_text.push_str(&exchange.response.body);
            stream
                .write_all(response_text.as_bytes())
                .expect("write response");
        }
    });
    format!("http://{}", addr)
}

fn serve_once<F>(assert_request: F, response: TestResponse) -> String
where
    F: Fn(String, Value) + Send + 'static,
{
    serve_sequence(vec![TestExchange {
        assert_request: Box::new(assert_request),
        response,
    }])
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let bytes_read = stream.read(&mut chunk).expect("read");
        if bytes_read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
        if let Some(split) = find_header_end(&buffer) {
            let header_text = String::from_utf8_lossy(&buffer[..split]).to_string();
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let body_len = buffer.len().saturating_sub(split + 4);
            if body_len >= content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

// --- Tests, one per `examples/<name>.rs` ---

/// Mirrors examples/quickstart.rs — keep in sync.
#[tokio::test]
async fn example_quickstart_chain() {
    let base_url = serve_once(
        |_request, json| {
            assert_eq!(json["system"], "Be concise.");
            assert_eq!(json["temperature"], 0.3);
            assert_eq!(json["messages"][0]["content"], "Why is the sky blue?");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "Rayleigh scattering."}],
                "usage": {"input_tokens": 9, "output_tokens": 3}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut c = anthropic("sk-test");
    c.provider.base_url = Some(base_url);

    let resp = c
        .text()
        .system("Be concise.")
        .temperature(0.3)
        .prompt("Why is the sky blue?")
        .await
        .expect("prompt succeeds");

    assert_eq!(resp.text, "Rayleigh scattering.");
    assert_eq!(resp.usage.input, 9);
    assert_eq!(resp.usage.output, 3);
}

/// Mirrors examples/agent.rs — keep in sync.
#[tokio::test]
async fn example_agent_chain() {
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|_request, json| {
                assert_eq!(json["messages"][0]["role"], "system");
                assert_eq!(json["messages"][0]["content"], "You are a calculator.");
                assert_eq!(json["messages"][1]["role"], "user");
                assert_eq!(json["messages"][1]["content"], "What is 2+3?");
                assert_eq!(json["tools"][0]["function"]["name"], "add");
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "choices": [{
                        "message": {
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {
                                    "name": "add",
                                    "arguments": "{\"a\":2,\"b\":3}"
                                }
                            }]
                        }
                    }],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5}
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|_request, json| {
                let messages = json["messages"].as_array().expect("messages array");
                assert_eq!(messages.len(), 4);
                assert_eq!(messages[3]["role"], "tool");
                assert_eq!(messages[3]["content"], "5");
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "choices": [{"message": {"content": "The sum is 5"}}],
                    "usage": {"prompt_tokens": 20, "completion_tokens": 5}
                })
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let mut c = openai("sk-test");
    c.provider.base_url = Some(base_url);

    let add = Tool::new(
        "add",
        "Add two numbers",
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            }
        }),
        |args| {
            let a = args["a"].as_f64().ok_or_else(|| "a not a number".to_string())?;
            let b = args["b"].as_f64().ok_or_else(|| "b not a number".to_string())?;
            Ok((a + b).to_string())
        },
    );

    let mut bot = c
        .agent()
        .system("You are a calculator.")
        .tool(add)
        .max_tool_iterations(5);

    let resp = bot.prompt("What is 2+3?").await.expect("tool loop succeeds");
    assert_eq!(resp.text, "The sum is 5");
}

/// Mirrors examples/streaming.rs — keep in sync.
#[tokio::test]
async fn example_streaming_chain() {
    let base_url = serve_once(
        |_request, json| {
            assert_eq!(json["stream"], true);
            assert_eq!(json["messages"][0]["role"], "system");
            assert_eq!(json["messages"][0]["content"], "Be brief");
            assert_eq!(json["messages"][1]["content"], "Tell me a joke");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"lo!\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{}}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_string(),
            headers: vec![("Content-Type", "text/event-stream")],
        },
    );

    let mut c = openai("sk-test");
    c.provider.base_url = Some(base_url);

    let mut chunks = Vec::new();
    let resp = c
        .text()
        .system("Be brief")
        .stream("Tell me a joke", |chunk| chunks.push(chunk.to_string()))
        .await
        .expect("stream succeeds");

    assert_eq!(chunks, vec!["Hel".to_string(), "lo!".to_string()]);
    assert_eq!(resp.text, "Hello!");
    assert_eq!(resp.usage.input, 5);
    assert_eq!(resp.usage.output, 2);
}

/// Mirrors examples/image.rs — keep in sync.
#[tokio::test]
async fn example_image_chain() {
    const FAKE_PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let encoded = base64::engine::general_purpose::STANDARD.encode(FAKE_PNG);

    let base_url = serve_once(
        |request, json| {
            assert!(request.contains("gemini-3.1-flash-image-preview:generateContent"));
            assert_eq!(
                json["generationConfig"]["imageConfig"]["aspectRatio"],
                "16:9"
            );
            assert_eq!(json["generationConfig"]["imageConfig"]["imageSize"], "2K");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "candidates": [{
                    "content": {
                        "parts": [
                            {"inlineData": {"mimeType": "image/png", "data": encoded}}
                        ]
                    }
                }],
                "usageMetadata": {"promptTokenCount": 12, "candidatesTokenCount": 1290}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut c = google("test-key");
    c.provider.base_url = Some(base_url);

    let img = c
        .image()
        .model("gemini-3.1-flash-image-preview")
        .aspect_ratio("16:9")
        .image_size("2K")
        .generate("A nano banana dish, studio lighting")
        .await
        .expect("generate succeeds");

    assert_eq!(img.images.len(), 1);
    assert_eq!(img.images[0].mime_type, "image/png");
    assert_eq!(img.images[0].data, FAKE_PNG);
}

/// Mirrors examples/upload.rs — keep in sync.
#[tokio::test]
async fn example_upload_chain() {
    let temp_path = std::env::temp_dir().join("llmkit-rust-example-upload-test.json");
    let data = br#"{"hello":"world"}"#;
    std::fs::write(&temp_path, data).expect("write temp file");

    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("POST /v1/files "));
                assert!(request.contains(
                    "name=\"file\"; filename=\"llmkit-rust-example-upload-test.json\""
                ));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "file_path_001",
                    "filename": "llmkit-rust-example-upload-test.json",
                    "purpose": "assistants"
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("POST /v1/files "));
                assert!(request.contains("name=\"file\"; filename=\"report.json\""));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "file_bytes_001",
                    "filename": "report.json",
                    "purpose": "assistants"
                })
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let mut c = openai("sk-test");
    c.provider.base_url = Some(base_url);

    let file_from_path = c
        .upload()
        .path(temp_path.to_string_lossy().to_string())
        .run()
        .await
        .expect("path upload succeeds");
    assert_eq!(file_from_path.id, "file_path_001");
    assert_eq!(file_from_path.name, "llmkit-rust-example-upload-test.json");

    let file_from_bytes = c
        .upload()
        .bytes(data.to_vec())
        .filename("report.json")
        .mime_type("application/json")
        .run()
        .await
        .expect("bytes upload succeeds");
    assert_eq!(file_from_bytes.id, "file_bytes_001");
    assert_eq!(file_from_bytes.name, "report.json");
}
