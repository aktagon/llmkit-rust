// Typed-builder smoke tests for the v1.0.0 surface (`llmkit::builders`).
//
// Ported from the legacy free-function tests in plan 019. Each test
// drives the same internal runtime through `c.text()` / `c.agent()` /
// `c.upload()` chains. `client.provider.base_url = Some(url)` is the
// supported way to redirect to a mock server; the `Client` exposes
// `provider` as a public field for exactly this reason (see
// `rust/tests/builders.rs`).
//
// Keep new tests in this file in the same shape — small chain, single
// terminal, mock-server roundtrip with `serve_once` / `serve_sequence`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use llmkit::builders::{anthropic, bedrock, google, grok, groq, new_client, openai};
use llmkit::{
    Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase, ProviderName, SafetySetting, Tool,
    HARM_BLOCK_THRESHOLD_NONE, HARM_CATEGORY_HARASSMENT,
};
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

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .system("You are helpful")
        .prompt("Hi")
        .await
        .expect("prompt succeeds");

    assert_eq!(response.text, "Hello!");
    assert_eq!(response.usage.input, 10);
    assert_eq!(response.usage.output, 5);
}

#[tokio::test]
async fn prompt_per_model_max_tokens_key_openai() {
    // BUG-001 / ADR-024: gpt-5 and the o-series emit max_completion_tokens;
    // gpt-4o keeps max_tokens. Per-model, same provider.
    let cases = [
        ("gpt-5", "max_completion_tokens", "max_tokens"),
        ("gpt-5-mini", "max_completion_tokens", "max_tokens"), // glob gpt-5*
        ("o3", "max_completion_tokens", "max_tokens"),
        ("o4-mini", "max_completion_tokens", "max_tokens"), // glob o*
        ("gpt-4o", "max_tokens", "max_completion_tokens"),  // unaffected
    ];
    for (model, want_key, wrong_key) in cases {
        let want_key = want_key.to_string();
        let wrong_key = wrong_key.to_string();
        let base_url = serve_once(
            move |_request, json| {
                assert_eq!(json[&want_key], 128, "model {model}: expected {want_key}=128");
                assert!(
                    json.get(&wrong_key).is_none(),
                    "model {model}: wrong key {wrong_key} leaked"
                );
            },
            TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "choices": [{"message": {"content": "ok"}}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1}
                })
                .to_string(),
                headers: vec![],
            },
        );
        let mut client = openai("test-key");
        client.provider.base_url = Some(base_url);
        client
            .text()
            .model(model)
            .max_tokens(128)
            .prompt("hi")
            .await
            .expect("prompt succeeds");
    }
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

    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .system("You are helpful")
        .prompt("Hi")
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

    let mut client = google("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .system("You are helpful")
        .temperature(0.2)
        .top_p(0.9)
        .max_tokens(77)
        .prompt("Hi")
        .await
        .expect("prompt succeeds");

    assert_eq!(response.text, "Hello from Gemini!");
    assert_eq!(response.usage.input, 9);
    assert_eq!(response.usage.output, 4);
}

#[tokio::test]
async fn invalid_reasoning_effort_is_rejected() {
    let mut client = openai("test-key");
    client.provider.base_url = Some("http://127.0.0.1:1".to_string());
    let error = client
        .text()
        .reasoning_effort("extreme")
        .prompt("Hi")
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

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .prompt("think hard")
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

    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let response = client.text().prompt("hi").await.expect("prompt succeeds");

    assert_eq!(response.usage.reasoning, 0);
}

#[tokio::test]
async fn prompt_surfaces_finish_reason() {
    // Anthropic emits stop_reason at the top level of the response. Verify
    // it lifts onto Response.finish_reason; Response.finish_message stays
    // empty because Anthropic has no equivalent free-text field.
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "truncated"}],
                "usage": {"input_tokens": 4, "output_tokens": 10},
                "stop_reason": "max_tokens",
            })
            .to_string(),
            headers: vec![],
        },
    );
    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .max_tokens(10)
        .prompt("ping")
        .await
        .expect("prompt succeeds");
    assert_eq!(response.finish_reason, "max_tokens");
    assert_eq!(response.finish_message, "");
}

#[tokio::test]
async fn prompt_omits_finish_reason_when_absent() {
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "ok"}],
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })
            .to_string(),
            headers: vec![],
        },
    );
    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let response = client.text().prompt("ping").await.expect("prompt succeeds");
    assert_eq!(response.finish_reason, "");
    assert_eq!(response.finish_message, "");
}

#[tokio::test]
async fn agent_caching_applies_to_request() {
    // BUG-004 / ADR-026: Agent::caching() must annotate the request body with
    // cache_control on every turn, exactly like the Text path. Before the
    // pipeline fix the agent builder dropped caching entirely.
    let base_url = serve_once(
        |_, json| {
            let system = json["system"].as_array().expect("system blocks (caching applied)");
            assert_eq!(system[0]["cache_control"]["type"], "ephemeral");
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "content": [{"type": "text", "text": "done"}],
                "usage": {"input_tokens": 2000, "output_tokens": 5}
            })
            .to_string(),
            headers: vec![],
        },
    );
    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().system("a long stable system prefix").caching();
    bot.prompt("hi").await.expect("agent cached prompt succeeds");
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

    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .system("You are helpful")
        .caching()
        .prompt("Hi")
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

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .system("You are helpful")
        .caching()
        .prompt("Hi")
        .await
        .expect("cached prompt succeeds");

    assert_eq!(response.usage.cache_read, 42);
}

#[tokio::test]
async fn prompt_with_caching_google_resource() {
    // The legacy variant set `.cache_ttl(90)`. The typed-builder v1.0.0
    // surface intentionally omits a `cache_ttl` chain method (the
    // ontology declares CacheTTL only as an api:SubOption under
    // api:Caching, not as a top-level FunctionalOption — see
    // ontology/api-mapping.ttl). Google's `default_ttl` of `"3600"` from
    // providers/generated/caching.rs is what flows through; the assertion
    // moves accordingly.
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|request, json| {
                assert!(request.starts_with("POST /v1beta/cachedContents?key=test-key "));
                assert_eq!(json["model"], "models/gemini-2.5-flash");
                assert_eq!(json["ttl"], "3600s");
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

    let mut client = google("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .system("You are helpful")
        .caching()
        .prompt("Hi")
        .await
        .expect("google cached prompt succeeds");

    assert_eq!(response.text, "cached gemini");
    assert_eq!(response.usage.cache_read, 33);
}

#[tokio::test]
async fn prompt_with_caching_unsupported() {
    let mut client = groq("test-key");
    client.provider.base_url = Some("http://127.0.0.1:1".to_string());
    let error = client
        .text()
        .caching()
        .prompt("Hi")
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

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let mut chunks = Vec::new();
    let response = client
        .text()
        .stream("Hi", |chunk| chunks.push(chunk.to_string()))
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

    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let mut chunks = Vec::new();
    let response = client
        .text()
        .stream("Hi", |chunk| chunks.push(chunk.to_string()))
        .await
        .expect("stream prompt succeeds");

    assert_eq!(response.text, "Hi there");
    assert_eq!(response.usage.input, 4);
    assert_eq!(response.usage.output, 3);
    assert_eq!(chunks, vec!["Hi".to_string(), " there".to_string()]);
}

// ADR-013: stream-time finish-reason surfaces on the returned Response.

#[tokio::test]
async fn stream_finish_reason_openai() {
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
                "data: [DONE]\n\n"
            )
            .to_string(),
            headers: vec![("Content-Type", "text/event-stream")],
        },
    );

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .stream("Hi", |_chunk| {})
        .await
        .expect("stream prompt succeeds");

    assert_eq!(response.finish_reason, "stop");
}

#[tokio::test]
async fn stream_finish_reason_anthropic() {
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: concat!(
                "event: content_block_delta\n",
                "data: {\"delta\":{\"text\":\"Hi\"}}\n\n",
                "event: message_delta\n",
                "data: {\"usage\":{\"output_tokens\":1}}\n\n",
                "event: message_stop\n",
                "data: {\"type\":\"message_stop\",\"stop_reason\":\"end_turn\"}\n\n"
            )
            .to_string(),
            headers: vec![("Content-Type", "text/event-stream")],
        },
    );

    let mut client = anthropic("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .stream("Hi", |_chunk| {})
        .await
        .expect("stream prompt succeeds");

    assert_eq!(response.finish_reason, "end_turn");
}

#[tokio::test]
async fn stream_finish_reason_google_filters_unspecified() {
    // First chunk carries FINISH_REASON_UNSPECIFIED — must NOT clobber the
    // terminal value (STOP) that arrives later.
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: concat!(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hi\"}]},\"finishReason\":\"FINISH_REASON_UNSPECIFIED\"}]}\n\n",
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"\"}]},\"finishReason\":\"STOP\"}]}\n\n"
            )
            .to_string(),
            headers: vec![("Content-Type", "text/event-stream")],
        },
    );

    let mut client = google("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .stream("Hi", |_chunk| {})
        .await
        .expect("stream prompt succeeds");

    assert_eq!(response.finish_reason, "STOP");
}

#[tokio::test]
async fn stream_finish_reason_empty_when_path_undeclared() {
    // Groq's A-Box declares no stream_finish_reason_path; even a frame
    // shaped like OpenAI's wire (with finish_reason populated) must NOT
    // produce a finish_reason on the Response.
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            )
            .to_string(),
            headers: vec![("Content-Type", "text/event-stream")],
        },
    );

    let mut client = groq("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .stream("Hi", |_chunk| {})
        .await
        .expect("stream prompt succeeds");

    assert_eq!(response.finish_reason, "");
}

#[tokio::test]
async fn stream_finish_reason_grok() {
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
                "data: [DONE]\n\n"
            )
            .to_string(),
            headers: vec![("Content-Type", "text/event-stream")],
        },
    );

    let mut client = grok("test-key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .stream("Hi", |_chunk| {})
        .await
        .expect("stream prompt succeeds");

    assert_eq!(response.finish_reason, "length");
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

    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    let results = client
        .text()
        .system("Be brief")
        .batch(vec!["Hello".to_string(), "World".to_string()])
        .await
        .expect("anthropic batch succeeds");

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].text, "response 1");
    assert_eq!(results[1].text, "response 2");
}

// ADR-012 REQ-PROP-003: every chain field set on the Text builder must
// propagate through Text::batch the same way it propagates through
// Text::prompt. Previously batch_inputs only forwarded middleware,
// silently dropping max_tokens / temperature / etc.
#[tokio::test]
async fn batch_propagates_chain_sampling_options() {
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|_, json| {
                let requests = json["requests"].as_array().expect("requests array");
                let params = &requests[0]["params"];
                assert_eq!(params["max_tokens"], 64);
                assert_eq!(params["temperature"], 0.3);
                assert_eq!(params["top_p"], 0.9);
                assert_eq!(params["stop_sequences"], serde_json::json!(["END"]));
                assert_eq!(params["system"], "be terse");
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "batch_opts",
                    "processing_status": "in_progress"
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("GET /v1/messages/batches/batch_opts "));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "id": "batch_opts",
                    "processing_status": "ended"
                })
                .to_string(),
                headers: vec![],
            },
        },
        TestExchange {
            assert_request: Box::new(|request, _| {
                assert!(request.starts_with("GET /v1/messages/batches/batch_opts/results "));
            }),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: concat!(
                    "{\"custom_id\":\"req-0\",\"result\":{\"type\":\"succeeded\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}],\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}}\n",
                )
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    let results = client
        .text()
        .system("be terse")
        .max_tokens(64)
        .temperature(0.3)
        .top_p(0.9)
        .stop_sequences(vec!["END".to_string()])
        .batch(vec!["ping".to_string()])
        .await
        .expect("anthropic batch succeeds");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "ok");
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

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let results = client
        .text()
        .system("Reply with only the word pong")
        .batch(vec!["ping".to_string(), "ping again".to_string()])
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

    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .schema(r#"{"type":"object","properties":{"color":{"type":"string"}}}"#)
        .prompt("color of sky")
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

    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .schema(r#"{"type":"object","properties":{"color":{"type":"string"}}}"#)
        .prompt("color of sky")
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

    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    let uploaded = client
        .upload()
        .path(temp_path.to_string_lossy().to_string())
        .run()
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

    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().system("You are a calculator").add_tool(Tool::new(
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

    let response = bot.prompt("What is 2+3?").await.expect("tool loop succeeds");

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

    let mut client = bedrock("AKID");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .model("test-model")
        .system("You are helpful")
        .max_tokens(12)
        .prompt("Hi")
        .await
        .expect("bedrock prompt succeeds");

    assert_eq!(response.text, "bedrock ok");
    assert_eq!(response.usage.input, 9);
    assert_eq!(response.usage.output, 4);
}

#[tokio::test]
async fn prompt_middleware_fires_pre_then_post() {
    let base_url = serve_once(
        |_request, _json| {},
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

    let calls: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let calls_for_mw = calls.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        calls_for_mw.lock().unwrap().push((ev.op, ev.phase));
        None
    });

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .add_middleware(vec![mw])
        .prompt("Hi")
        .await
        .expect("prompt succeeds");

    let recorded = calls.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert!(matches!(recorded[0].0, MiddlewareOp::LlmRequest));
    assert!(matches!(recorded[0].1, MiddlewarePhase::Pre));
    assert!(matches!(recorded[1].1, MiddlewarePhase::Post));
}

#[tokio::test]
async fn prompt_middleware_can_veto() {
    let mw: MiddlewareFn = Arc::new(|ev: &Event| {
        if matches!(ev.phase, MiddlewarePhase::Pre) {
            Some("veto".into())
        } else {
            None
        }
    });

    let mut client = openai("k");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .text()
        .add_middleware(vec![mw])
        .prompt("Hi")
        .await;
    match result {
        Err(llmkit::Error::MiddlewareVeto(_)) => {}
        other => panic!("expected MiddlewareVeto, got {:?}", other),
    }
}

#[tokio::test]
async fn upload_middleware_fires_pre_then_post() {
    let temp_path = std::env::temp_dir().join("llmkit-rust-upload-mw.json");
    std::fs::write(&temp_path, br#"{"a":1}"#).expect("write temp file");

    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({"id": "file_x", "filename": "x"}).to_string(),
            headers: vec![],
        },
    );

    let calls: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let calls_for_mw = calls.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        calls_for_mw.lock().unwrap().push((ev.op, ev.phase));
        None
    });

    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .upload()
        .path(temp_path.to_string_lossy().to_string())
        .add_middleware(vec![mw])
        .run()
        .await
        .expect("upload succeeds");

    let recorded = calls.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert!(matches!(recorded[0].0, MiddlewareOp::Upload));
    assert!(matches!(recorded[0].1, MiddlewarePhase::Pre));
    assert!(matches!(recorded[1].1, MiddlewarePhase::Post));
}

#[tokio::test]
async fn upload_middleware_can_veto() {
    let temp_path = std::env::temp_dir().join("llmkit-rust-upload-veto.json");
    std::fs::write(&temp_path, br#"{}"#).expect("write temp file");

    let mw: MiddlewareFn = Arc::new(|ev: &Event| {
        if matches!(ev.phase, MiddlewarePhase::Pre) {
            Some("nope".into())
        } else {
            None
        }
    });

    let mut client = openai("k");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .upload()
        .path(temp_path.to_string_lossy().to_string())
        .add_middleware(vec![mw])
        .run()
        .await;
    match result {
        Err(llmkit::Error::MiddlewareVeto(_)) => {}
        other => panic!("expected MiddlewareVeto, got {:?}", other),
    }
}

#[tokio::test]
async fn submit_batch_middleware_fires_pre_then_post() {
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "id": "batch_mw_test",
                "processing_status": "in_progress"
            })
            .to_string(),
            headers: vec![],
        },
    );

    let calls: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let calls_for_mw = calls.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        calls_for_mw.lock().unwrap().push((ev.op, ev.phase));
        None
    });

    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    let handle = client
        .text()
        .system("Be brief")
        .add_middleware(vec![mw])
        .submit_batch(vec!["Hi".to_string()])
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "batch_mw_test");

    let recorded = calls.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert!(matches!(recorded[0].0, MiddlewareOp::BatchSubmit));
    assert!(matches!(recorded[0].1, MiddlewarePhase::Pre));
    assert!(matches!(recorded[1].1, MiddlewarePhase::Post));
}

#[tokio::test]
async fn submit_batch_middleware_can_veto() {
    let mw: MiddlewareFn = Arc::new(|ev: &Event| {
        if matches!(ev.phase, MiddlewarePhase::Pre) {
            Some("nope".into())
        } else {
            None
        }
    });

    let mut client = anthropic("k");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .text()
        .add_middleware(vec![mw])
        .submit_batch(vec!["Hi".to_string()])
        .await;
    match result {
        Err(llmkit::Error::MiddlewareVeto(_)) => {}
        other => panic!("expected MiddlewareVeto, got {:?}", other),
    }
}

#[tokio::test]
async fn agent_middleware_fires_llm_and_tool_call() {
    let base_url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|_, _| {}),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "choices": [{
                        "message": {
                            "tool_calls": [{
                                "id": "call_1",
                                "type": "function",
                                "function": {"name": "add", "arguments": "{\"a\":2,\"b\":3}"}
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
            assert_request: Box::new(|_, _| {}),
            response: TestResponse {
                status_line: "HTTP/1.1 200 OK",
                body: serde_json::json!({
                    "choices": [{"message": {"content": "5"}}],
                    "usage": {"prompt_tokens": 20, "completion_tokens": 5}
                })
                .to_string(),
                headers: vec![],
            },
        },
    ]);

    let calls: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let calls_for_mw = calls.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        calls_for_mw.lock().unwrap().push((ev.op, ev.phase));
        None
    });

    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client
        .agent()
        .system("You add numbers")
        .add_middleware(vec![mw])
        .add_tool(Tool::new(
            "add",
            "add two numbers",
            serde_json::json!({"type": "object"}),
            |args| {
                let a = args.get("a").and_then(|v| v.as_i64()).unwrap_or(0);
                let b = args.get("b").and_then(|v| v.as_i64()).unwrap_or(0);
                Ok((a + b).to_string())
            },
        ));

    bot.prompt("2+3?").await.expect("chat succeeds");

    let recorded = calls.lock().unwrap().clone();
    // 2 LLM turns (pre+post each = 4) + 1 tool call (pre+post = 2) = 6 events.
    assert_eq!(recorded.len(), 6);
    let ops: Vec<MiddlewareOp> = recorded.iter().map(|(op, _)| *op).collect();
    let llm_count = ops.iter().filter(|op| matches!(op, MiddlewareOp::LlmRequest)).count();
    let tool_count = ops.iter().filter(|op| matches!(op, MiddlewareOp::ToolCall)).count();
    assert_eq!(llm_count, 4);
    assert_eq!(tool_count, 2);
}

#[tokio::test]
async fn agent_middleware_can_veto_tool() {
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{
                    "message": {
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "add", "arguments": "{}"}
                        }]
                    }
                }],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mw: MiddlewareFn = Arc::new(|ev: &Event| {
        if matches!(ev.op, MiddlewareOp::ToolCall) && matches!(ev.phase, MiddlewarePhase::Pre) {
            Some("tool blocked".into())
        } else {
            None
        }
    });

    let mut client = new_client(ProviderName::OpenAI, "key");
    client.provider.base_url = Some(base_url);
    let mut bot = client
        .agent()
        .system("system")
        .add_middleware(vec![mw])
        .add_tool(Tool::new(
            "add",
            "add",
            serde_json::json!({"type": "object"}),
            |_| Ok("10".into()),
        ));

    let result = bot.prompt("hello").await;
    match result {
        Err(llmkit::Error::MiddlewareVeto(_)) => {}
        other => panic!("expected MiddlewareVeto on tool call, got {:?}", other),
    }
}

#[tokio::test]
async fn usage_cost_openrouter() {
    // BUG-005 / ADR-027: OpenRouter reports usage.cost (USD) -> Usage.cost.
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "ok"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "cost": 0.00042}
            })
            .to_string(),
            headers: vec![],
        },
    );
    let mut client = new_client(ProviderName::Openrouter, "k");
    client.provider.base_url = Some(base_url);
    let resp = client.text().prompt("hi").await.expect("prompt succeeds");
    assert_eq!(resp.usage.cost, 0.00042);
}

#[tokio::test]
async fn usage_cost_zero_for_no_cost_provider() {
    // OpenAI declares no usage_cost_path, so a stray cost field is ignored.
    let base_url = serve_once(
        |_, _| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "ok"}}],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "cost": 0.99}
            })
            .to_string(),
            headers: vec![],
        },
    );
    let mut client = openai("k");
    client.provider.base_url = Some(base_url);
    let resp = client.text().prompt("hi").await.expect("prompt succeeds");
    assert_eq!(resp.usage.cost, 0.0);
}

#[tokio::test]
async fn agent_google_tool_uses_parameters_json_schema() {
    // ADR-025 / BUG-002: Google tool params go under parametersJsonSchema
    // (native JSON Schema slot), passed verbatim — additionalProperties and
    // ["string","null"] unions survive. The mock returns a plain text turn so
    // the agent stops after the single request the assertion inspects.
    let base_url = serve_once(
        |_request, json| {
            let decls = json["tools"][0]["functionDeclarations"]
                .as_array()
                .expect("functionDeclarations array");
            let params = &decls[0]["parametersJsonSchema"];
            assert_eq!(params["additionalProperties"], false, "verbatim additionalProperties");
            assert_eq!(params["properties"]["note"]["type"][0], "string", "union preserved");
            assert!(
                decls[0].get("parameters").is_none(),
                "must not use the OpenAPI-subset 'parameters' field for Google"
            );
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "candidates": [{"content": {"parts": [{"text": "done"}]}}],
                "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut client = google("g-key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().add_tool(Tool::new(
        "annotate",
        "annotate a value",
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"note": {"type": ["string", "null"]}}
        }),
        |_| Ok("ok".into()),
    ));
    bot.prompt("hi").await.expect("google tool turn succeeds");
}

#[tokio::test]
async fn prompt_google_safety_settings_wire_body() {
    let base_url = serve_once(
        |_request, json| {
            let ss = json["safetySettings"].as_array().expect("safetySettings array");
            assert_eq!(ss.len(), 1);
            assert_eq!(ss[0]["category"], HARM_CATEGORY_HARASSMENT);
            assert_eq!(ss[0]["threshold"], HARM_BLOCK_THRESHOLD_NONE);
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "candidates": [{"content": {"parts": [{"text": "ok"}]}}],
                "usageMetadata": {"promptTokenCount": 1, "candidatesTokenCount": 1}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut client = google("test-key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .safety_settings(vec![SafetySetting {
            category: HARM_CATEGORY_HARASSMENT.into(),
            threshold: HARM_BLOCK_THRESHOLD_NONE.into(),
        }])
        .prompt("hello")
        .await
        .expect("prompt succeeds");
}

#[tokio::test]
async fn prompt_openai_safety_settings_silently_dropped() {
    let base_url = serve_once(
        |_request, json| {
            assert!(
                json.get("safetySettings").is_none(),
                "safetySettings must not appear in OpenAI wire body"
            );
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "ok"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let resp = client
        .text()
        .safety_settings(vec![SafetySetting {
            category: HARM_CATEGORY_HARASSMENT.into(),
            threshold: HARM_BLOCK_THRESHOLD_NONE.into(),
        }])
        .prompt("hello")
        .await
        .expect("prompt succeeds");
    assert_eq!(resp.text, "ok");
}

// ADR-014: `.raw()` populates Response.raw with the parsed provider
// body; absence leaves it None.
#[tokio::test]
async fn raw_populates_response_raw_when_chain_method_set() {
    let base_url = serve_once(
        |_request, _json| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "id": "chatcmpl-1",
                "choices": [{"message": {"content": "hi"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1},
                "x_provider_extra": 7
            })
            .to_string(),
            headers: vec![],
        },
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let resp = client
        .text()
        .raw()
        .prompt("hi")
        .await
        .expect("prompt succeeds");
    let raw = resp.raw.as_ref().expect("raw populated");
    assert_eq!(raw.get("x_provider_extra").and_then(Value::as_i64), Some(7));
}

#[tokio::test]
async fn raw_absent_leaves_response_raw_none() {
    let base_url = serve_once(
        |_request, _json| {},
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "hi"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1}
            })
            .to_string(),
            headers: vec![],
        },
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(base_url);
    let resp = client
        .text()
        .prompt("hi")
        .await
        .expect("prompt succeeds");
    assert!(resp.raw.is_none(), "Response.raw: got {:?}, want None", resp.raw);
}
