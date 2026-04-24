use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock};
use std::net::TcpListener;
use std::thread;

use llmkit::{prompt, prompt_batch, prompt_stream, upload_file, Agent, PromptOptions, Provider, ProviderName, Request, Tool};
use serde_json::Value;

struct TestResponse {
    status_line: &'static str,
    body: String,
    headers: Vec<(&'static str, &'static str)>,
}

struct TestExchange {
    assert_request: Box<dyn Fn(String, Value) + Send + 'static>,
    response: TestResponse,
}

fn aws_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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

#[tokio::test]
async fn prompt_openai_shape() {
    let base_url = serve_once(
        |request, json| {
            let request_lower = request.to_lowercase();
            assert!(request_lower.contains("authorization: bearer test-key\r\n"));
            assert_eq!(json["model"], "gpt-4o-2024-08-06");
            let messages = json["messages"].as_array().expect("messages array");
            assert_eq!(messages[0]["role"], "system");
            assert_eq!(messages[0]["content"], "You are helpful");
            assert_eq!(messages[1]["role"], "user");
            assert_eq!(messages[1]["content"], "Hi");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "Hello!"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Openai, "test-key").with_base_url(base_url),
        &Request::new("Hi").with_system("You are helpful"),
        PromptOptions::new(),
    )
    .await
    .expect("prompt succeeds");

    assert_eq!(response.text, "Hello!");
    assert_eq!(response.usage.input, 10);
    assert_eq!(response.usage.output, 5);
}

#[tokio::test]
async fn prompt_anthropic_shape() {
    let base_url = serve_once(
        |request, json| {
            let request_lower = request.to_lowercase();
            assert!(request_lower.contains("x-api-key: test-key\r\n"));
            assert!(request_lower.contains("anthropic-version: 2023-06-01\r\n"));
            assert_eq!(json["system"], "You are helpful");
            let messages = json["messages"].as_array().expect("messages array");
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0]["role"], "user");
            assert_eq!(messages[0]["content"], "Hi");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "Hello from Claude!"}],
                "usage": {"input_tokens": 12, "output_tokens": 7}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Anthropic, "test-key").with_base_url(base_url),
        &Request::new("Hi").with_system("You are helpful"),
        PromptOptions::new(),
    )
    .await
    .expect("prompt succeeds");

    assert_eq!(response.text, "Hello from Claude!");
    assert_eq!(response.usage.input, 12);
    assert_eq!(response.usage.output, 7);
}

#[tokio::test]
async fn prompt_google_shape_and_wrapped_options() {
    let base_url = serve_once(
        |request, json| {
            assert!(request.starts_with("POST /v1beta/models/gemini-2.5-flash:generateContent?key=test-key "));
            assert_eq!(json.get("model"), None);
            assert_eq!(json["system_instruction"]["parts"][0]["text"], "You are helpful");
            assert_eq!(json["contents"][0]["role"], "user");
            assert_eq!(json["contents"][0]["parts"][0]["text"], "Hi");
            assert_eq!(json["generationConfig"]["temperature"], 0.2);
            assert_eq!(json["generationConfig"]["top_p"], 0.9);
            assert_eq!(json["generationConfig"]["max_output_tokens"], 77);
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "candidates": [{"content": {"parts": [{"text": "Hello from Gemini!"}]}}],
                "usageMetadata": {"promptTokenCount": 9, "candidatesTokenCount": 4}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Google, "test-key").with_base_url(base_url),
        &Request::new("Hi").with_system("You are helpful"),
        PromptOptions::new().temperature(0.2).top_p(0.9).max_tokens(77),
    )
    .await
    .expect("prompt succeeds");

    assert_eq!(response.text, "Hello from Gemini!");
    assert_eq!(response.usage.input, 9);
    assert_eq!(response.usage.output, 4);
}

#[tokio::test]
async fn invalid_reasoning_effort_is_rejected() {
    let error = prompt(
        &Provider::new(ProviderName::Openai, "test-key").with_base_url("http://127.0.0.1:1"),
        &Request::new("Hi"),
        PromptOptions::new().reasoning_effort("extreme"),
    )
    .await
    .expect_err("invalid option should fail before http");

    let message = error.to_string();
    assert!(message.contains("reasoning_effort"));
}

#[tokio::test]
async fn prompt_populates_reasoning_tokens_for_openai() {
    // o1/o3/o4 models expose usage.completion_tokens_details.reasoning_tokens.
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "reasoned"}}],
                "usage": {
                    "prompt_tokens": 40,
                    "completion_tokens": 25,
                    "completion_tokens_details": {"reasoning_tokens": 17}
                }
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Openai, "test-key").with_base_url(base_url),
        &Request::new("think hard"),
        PromptOptions::new(),
    )
    .await
    .expect("prompt succeeds");

    assert_eq!(response.usage.reasoning, 17);
    assert_eq!(response.usage.input, 40);
    assert_eq!(response.usage.output, 25);
}

#[tokio::test]
async fn prompt_reasoning_zero_for_unreported_provider() {
    // Anthropic bundles thinking into output_tokens; Usage.reasoning stays 0.
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "hi"}],
                "usage": {"input_tokens": 5, "output_tokens": 3}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Anthropic, "test-key").with_base_url(base_url),
        &Request::new("hi"),
        PromptOptions::new(),
    )
    .await
    .expect("prompt succeeds");

    assert_eq!(response.usage.reasoning, 0);
}

#[tokio::test]
async fn prompt_with_caching_anthropic() {
    let base_url = serve_once(
        |_, json| {
            let system = json["system"].as_array().expect("system blocks");
            assert_eq!(system[0]["type"], "text");
            assert_eq!(system[0]["text"], "You are helpful");
            assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "cached!"}],
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 5,
                    "cache_creation_input_tokens": 100,
                    "cache_read_input_tokens": 0
                }
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Anthropic, "test-key").with_base_url(base_url),
        &Request::new("Hi").with_system("You are helpful"),
        PromptOptions::new().caching(),
    )
    .await
    .expect("cached prompt succeeds");

    assert_eq!(response.text, "cached!");
    assert_eq!(response.usage.cache_write, 100);
    assert_eq!(response.usage.cache_read, 0);
}

#[tokio::test]
async fn prompt_with_caching_openai() {
    let base_url = serve_once(
        |_, json| {
            let messages = json["messages"].as_array().expect("messages");
            assert!(messages[0]["content"].is_string());
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "ok"}}],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 5,
                    "prompt_tokens_details": {"cached_tokens": 42}
                }
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Openai, "test-key").with_base_url(base_url),
        &Request::new("Hi").with_system("You are helpful"),
        PromptOptions::new().caching(),
    )
    .await
    .expect("cached prompt succeeds");

    assert_eq!(response.usage.cache_read, 42);
}

#[tokio::test]
async fn prompt_with_caching_google_resource() {
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|request, json| {
                assert!(request.starts_with("POST /v1beta/cachedContents?key=test-key "));
                assert_eq!(json["model"], "models/gemini-2.5-flash");
                assert_eq!(json["ttl"], "90s");
                assert_eq!(json["systemInstruction"]["parts"][0]["text"], "You are helpful");
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({"name": "cachedContents/abc"}).to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, json| {
                assert!(request.starts_with("POST /v1beta/models/gemini-2.5-flash:generateContent?key=test-key "));
                assert_eq!(json["cachedContent"], "cachedContents/abc");
                assert!(json.get("system_instruction").is_none());
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "candidates": [{"content": {"parts": [{"text": "cached gemini"}]}}],
                    "usageMetadata": {
                        "promptTokenCount": 9,
                        "candidatesTokenCount": 4,
                        "cachedContentTokenCount": 33
                    }
                })
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let response = prompt(
        &Provider::new(ProviderName::Google, "test-key").with_base_url(base_url),
        &Request::new("Hi").with_system("You are helpful"),
        PromptOptions::new().caching().cache_ttl(90),
    )
    .await
    .expect("google cached prompt succeeds");

    assert_eq!(response.text, "cached gemini");
    assert_eq!(response.usage.cache_read, 33);
}

#[tokio::test]
async fn prompt_with_caching_unsupported() {
    let error = prompt(
        &Provider::new(ProviderName::Groq, "test-key").with_base_url("http://127.0.0.1:1"),
        &Request::new("Hi"),
        PromptOptions::new().caching(),
    )
    .await
    .expect_err("unsupported caching should fail");

    assert!(error.to_string().contains("caching"));
}

#[tokio::test]
async fn prompt_stream_openai_shape() {
    let base_url = serve_once(
        |request, json| {
            let request_lower = request.to_lowercase();
            assert!(request_lower.contains("authorization: bearer test-key\r\n"));
            assert_eq!(json["stream"], true);
            assert_eq!(json["messages"][0]["content"], "Hi");
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

    let mut chunks = Vec::new();
    let response = prompt_stream(
        &Provider::new(ProviderName::Openai, "test-key").with_base_url(base_url),
        &Request::new("Hi"),
        PromptOptions::new(),
        |chunk| chunks.push(chunk.to_string()),
    )
    .await
    .expect("stream prompt succeeds");

    assert_eq!(response.text, "Hello!");
    assert_eq!(response.usage.input, 5);
    assert_eq!(response.usage.output, 2);
    assert_eq!(chunks, vec!["Hel".to_string(), "lo!".to_string()]);
}

#[tokio::test]
async fn prompt_stream_anthropic_shape() {
    let base_url = serve_once(
        |request, json| {
            let request_lower = request.to_lowercase();
            assert!(request_lower.contains("x-api-key: test-key\r\n"));
            assert!(request_lower.contains("anthropic-version: 2023-06-01\r\n"));
            assert_eq!(json["stream"], true);
            assert_eq!(json["messages"][0]["content"], "Hi");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: concat!(
                "event: content_block_delta\n",
                "data: {\"delta\":{\"text\":\"Hi\"}}\n\n",
                "event: content_block_delta\n",
                "data: {\"delta\":{\"text\":\" there\"}}\n\n",
                "event: message_delta\n",
                "data: {\"usage\":{\"input_tokens\":4,\"output_tokens\":3}}\n\n",
                "event: message_stop\n",
                "data: {}\n\n"
            )
            .to_string(),
            headers: vec![("Content-Type", "text/event-stream")],
        },
    );

    let mut chunks = Vec::new();
    let response = prompt_stream(
        &Provider::new(ProviderName::Anthropic, "test-key").with_base_url(base_url),
        &Request::new("Hi"),
        PromptOptions::new(),
        |chunk| chunks.push(chunk.to_string()),
    )
    .await
    .expect("stream prompt succeeds");

    assert_eq!(response.text, "Hi there");
    assert_eq!(response.usage.input, 4);
    assert_eq!(response.usage.output, 3);
    assert_eq!(chunks, vec!["Hi".to_string(), " there".to_string()]);
}

#[tokio::test]
async fn prompt_batch_anthropic() {
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|_, json| {
                let requests = json["requests"].as_array().expect("requests array");
                assert_eq!(requests.len(), 2);
                assert_eq!(requests[0]["custom_id"], "req-0");
                assert!(requests[0]["params"].is_object());
                assert_eq!(requests[1]["custom_id"], "req-1");
                assert!(requests[1]["params"].is_object());
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "batch_123",
                    "processing_status": "in_progress"
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("GET /v1/messages/batches/batch_123 "));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "batch_123",
                    "processing_status": "ended"
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("GET /v1/messages/batches/batch_123/results "));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: concat!(
                    "{\"custom_id\":\"req-0\",\"result\":{\"type\":\"succeeded\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"response 1\"}],\"usage\":{\"input_tokens\":5,\"output_tokens\":3}}}}\n",
                    "{\"custom_id\":\"req-1\",\"result\":{\"type\":\"succeeded\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"response 2\"}],\"usage\":{\"input_tokens\":7,\"output_tokens\":4}}}}\n"
                )
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let results = prompt_batch(
        &Provider::new(ProviderName::Anthropic, "key").with_base_url(base_url),
        &[
            Request::new("Hello").with_system("Be brief"),
            Request::new("World").with_system("Be brief"),
        ],
        PromptOptions::new(),
    )
    .await
    .expect("anthropic batch succeeds");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].text, "response 1");
    assert_eq!(results[1].text, "response 2");
}

#[tokio::test]
async fn prompt_batch_openai() {
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("POST /v1/files "));
                assert!(request.contains("name=\"purpose\""));
                assert!(request.contains("batch"));
                assert!(request.contains("name=\"file\"; filename=\"batch_input.jsonl\""));
                assert!(request.contains("\"custom_id\":\"req-0\""));
                assert!(request.contains("\"custom_id\":\"req-1\""));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({"id": "file-abc123"}).to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|_, json| {
                assert_eq!(json["input_file_id"], "file-abc123");
                assert_eq!(json["endpoint"], "/v1/chat/completions");
                assert_eq!(json["completion_window"], "24h");
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({"id": "batch_xyz", "status": "validating"}).to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("GET /v1/batches/batch_xyz "));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "batch_xyz",
                    "status": "completed",
                    "output_file_id": "file-out456"
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("GET /v1/batches/batch_xyz "));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "batch_xyz",
                    "status": "completed",
                    "output_file_id": "file-out456"
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("GET /v1/files/file-out456/content "));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: concat!(
                    "{\"custom_id\":\"req-0\",\"response\":{\"status_code\":200,\"body\":{\"choices\":[{\"message\":{\"content\":\"pong 1\"}}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3}}}}\n",
                    "{\"custom_id\":\"req-1\",\"response\":{\"status_code\":200,\"body\":{\"choices\":[{\"message\":{\"content\":\"pong 2\"}}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":4}}}}\n"
                )
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let results = prompt_batch(
        &Provider::new(ProviderName::Openai, "test-key").with_base_url(base_url),
        &[
            Request::new("ping").with_system("Reply with only the word pong"),
            Request::new("ping again").with_system("Reply with only the word pong"),
        ],
        PromptOptions::new(),
    )
    .await
    .expect("openai batch succeeds");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].text, "pong 1");
    assert_eq!(results[0].usage.input, 5);
    assert_eq!(results[1].text, "pong 2");
}

#[tokio::test]
async fn structured_output_openai() {
    let base_url = serve_once(
        |_, json| {
            let response_format = json["response_format"].as_object().expect("response_format");
            assert_eq!(response_format["type"], "json_schema");
            let json_schema = response_format["json_schema"].as_object().expect("json_schema");
            assert_eq!(json_schema["strict"], true);
            assert!(json_schema.get("schema").is_some());
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "{\"color\":\"blue\"}"}}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Openai, "key").with_base_url(base_url),
        &Request::new("color of sky")
            .with_schema(r#"{"type":"object","properties":{"color":{"type":"string"}}}"#),
        PromptOptions::new(),
    )
    .await
    .expect("structured output prompt succeeds");

    assert_eq!(response.text, r#"{"color":"blue"}"#);
}

#[tokio::test]
async fn structured_output_anthropic() {
    let base_url = serve_once(
        |request, json| {
            let output_format = json["output_format"].as_object().expect("output_format");
            assert_eq!(output_format["type"], "json_schema");
            assert!(output_format.get("schema").is_some());
            let request_lower = request.to_lowercase();
            assert!(request_lower.contains("anthropic-beta: structured-outputs-2025-11-13\r\n"));
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "{\"color\":\"blue\"}"}],
                "usage": {"input_tokens": 5, "output_tokens": 3}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Anthropic, "key").with_base_url(base_url),
        &Request::new("color of sky")
            .with_schema(r#"{"type":"object","properties":{"color":{"type":"string"}}}"#),
        PromptOptions::new(),
    )
    .await
    .expect("structured output prompt succeeds");

    assert_eq!(response.text, r#"{"color":"blue"}"#);
}

#[tokio::test]
async fn upload_file_openai() {
    let temp_path = std::env::temp_dir().join("llmkit-rust-upload.json");
    std::fs::write(&temp_path, br#"{"hello":"world"}"#).expect("write temp file");

    let base_url = serve_once(
        |request, _| {
            assert!(request.starts_with("POST /v1/files "));
            assert!(request.contains("name=\"purpose\""));
            assert!(request.contains("assistants"));
            assert!(request.contains("name=\"file\"; filename=\"llmkit-rust-upload.json\""));
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "id": "file_123",
                "filename": "llmkit-rust-upload.json",
                "purpose": "assistants"
            })
            .to_string(),
            headers: vec![],
        },
    );

    let uploaded = upload_file(
        &Provider::new(ProviderName::Openai, "key").with_base_url(base_url),
        &temp_path,
    )
    .await
    .expect("upload succeeds");

    assert_eq!(uploaded.id, "file_123");
    assert_eq!(uploaded.name, "llmkit-rust-upload.json");
}

#[tokio::test]
async fn agent_with_tools_openai() {
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|request, json| {
                let request_lower = request.to_lowercase();
                assert!(request_lower.contains("authorization: bearer key\r\n"));
                assert_eq!(json["messages"][0]["role"], "system");
                assert_eq!(json["messages"][0]["content"], "You are a calculator");
                assert_eq!(json["messages"][1]["role"], "user");
                assert_eq!(json["messages"][1]["content"], "What is 2+3?");
                assert_eq!(json["tools"][0]["type"], "function");
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
            assert_request: Box::new(|_, json| {
                let messages = json["messages"].as_array().expect("messages");
                assert_eq!(messages.len(), 4);
                assert_eq!(messages[0]["role"], "system");
                assert_eq!(messages[1]["role"], "user");
                assert_eq!(messages[2]["role"], "assistant");
                assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "add");
                assert_eq!(messages[3]["role"], "tool");
                assert_eq!(messages[3]["tool_call_id"], "call_1");
                assert_eq!(messages[3]["content"], "5");
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "choices": [{
                        "message": {"content": "The sum is 5"}
                    }],
                    "usage": {"prompt_tokens": 20, "completion_tokens": 5}
                })
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let mut agent = Agent::new(Provider::new(ProviderName::Openai, "key").with_base_url(base_url));
    agent.set_system("You are a calculator");
    agent.add_tool(Tool::new(
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
            let a = args["a"].as_f64().expect("a");
            let b = args["b"].as_f64().expect("b");
            Ok((a + b).to_string())
        },
    ));

    let response = agent.chat("What is 2+3?").await.expect("tool loop succeeds");

    assert_eq!(response.text, "The sum is 5");
    assert_eq!(response.usage.input, 30);
    assert_eq!(response.usage.output, 10);
}

#[tokio::test]
async fn prompt_bedrock_sigv4_shape() {
    let _guard = aws_env_lock().lock().expect("lock");
    std::env::set_var("AWS_REGION", "us-east-1");
    std::env::set_var("AWS_SECRET_ACCESS_KEY", "SECRET");
    std::env::set_var("AWS_SESSION_TOKEN", "SESSION");

    let base_url = serve_once(
        |request, json| {
            let request_lower = request.to_lowercase();
            assert!(request.starts_with("POST /model/test-model/converse "));
            assert!(request_lower.contains("authorization: aws4-hmac-sha256 "));
            assert!(request_lower.contains("x-amz-date: "));
            assert!(request_lower.contains("x-amz-content-sha256: "));
            assert!(request_lower.contains("x-amz-security-token: session\r\n"));
            assert_eq!(json["system"][0]["text"], "You are helpful");
            assert_eq!(json["messages"][0]["role"], "user");
            assert_eq!(json["messages"][0]["content"][0]["text"], "Hi");
            assert_eq!(json["inferenceConfig"]["maxTokens"], 12);
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "output": {
                    "message": {
                        "content": [{"text": "bedrock ok"}]
                    }
                },
                "usage": {"inputTokens": 9, "outputTokens": 4}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let response = prompt(
        &Provider::new(ProviderName::Bedrock, "AKID")
            .with_base_url(base_url)
            .with_model("test-model"),
        &Request::new("Hi").with_system("You are helpful"),
        PromptOptions::new().max_tokens(12),
    )
    .await
    .expect("bedrock prompt succeeds");

    assert_eq!(response.text, "bedrock ok");
    assert_eq!(response.usage.input, 9);
    assert_eq!(response.usage.output, 4);
}
