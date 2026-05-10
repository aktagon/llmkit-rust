//! Phase 2b smoke tests for llmkit::builders.
//!
//! Exercises every public symbol — chains, terminal stubs, type
//! aliases, per-provider factories — so the eventual strict Rust
//! coverage gate sees full function coverage on builders.rs.

use llmkit::builders::{
    ai21, anthropic, azure, bedrock, cerebras, cohere, deepseek, doubao, ernie, fireworks, google,
    grok, groq, lmstudio, minimax, mistral, moonshot, new_client, ollama, openai, openrouter,
    perplexity, qwen, sambanova, together, vllm, yi, zhipu, Agent, Client, Image, ImageData,
    MediaRef, Text, Upload,
};
use llmkit::ProviderName;

// Field-access tests (text/image/agent/upload chain landings,
// client_text_method_returns_fresh_builder, agent state forking) moved
// to crate-internal `src/builders/internal_tests.rs` when builder
// fields were locked down to `pub(crate)` in plan 020.

#[test]
fn every_per_provider_factory_constructs_client() {
    let pairs: Vec<(&str, fn(&str) -> Client)> = vec![
        ("ai21", |k| ai21(k)),
        ("anthropic", |k| anthropic(k)),
        ("azure", |k| azure(k)),
        ("bedrock", |k| bedrock(k)),
        ("cerebras", |k| cerebras(k)),
        ("cohere", |k| cohere(k)),
        ("deepseek", |k| deepseek(k)),
        ("doubao", |k| doubao(k)),
        ("ernie", |k| ernie(k)),
        ("fireworks", |k| fireworks(k)),
        ("google", |k| google(k)),
        ("grok", |k| grok(k)),
        ("groq", |k| groq(k)),
        ("lmstudio", |k| lmstudio(k)),
        ("minimax", |k| minimax(k)),
        ("mistral", |k| mistral(k)),
        ("moonshot", |k| moonshot(k)),
        ("ollama", |k| ollama(k)),
        ("openai", |k| openai(k)),
        ("openrouter", |k| openrouter(k)),
        ("perplexity", |k| perplexity(k)),
        ("qwen", |k| qwen(k)),
        ("sambanova", |k| sambanova(k)),
        ("together", |k| together(k)),
        ("vllm", |k| vllm(k)),
        ("yi", |k| yi(k)),
        ("zhipu", |k| zhipu(k)),
    ];
    assert_eq!(pairs.len(), 27);
    for (_label, factory) in pairs {
        let c = factory("k");
        assert_eq!(c.provider.api_key, "k");
    }
    // Generic escape hatch.
    let c = new_client(ProviderName::OpenAI, "k");
    assert_eq!(c.provider.api_key, "k");
}

// === Phase 3 wiring verification ===

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Tiny single-shot HTTP server: reads one request, returns the
/// canned response, exits. Returns the listener URL.
fn mock_server(response_body: String) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buffer = vec![0u8; 8192];
        let mut total = Vec::new();
        loop {
            let n = stream.read(&mut buffer).unwrap_or(0);
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buffer[..n]);
            // Look for content-length and end of body. Naive: stop at
            // first read that contains \r\n\r\n followed by enough bytes.
            if let Some(headers_end) = total
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
            {
                let header = String::from_utf8_lossy(&total[..headers_end]);
                let cl = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if total.len() >= headers_end + 4 + cl {
                    break;
                }
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    format!("http://{}", addr)
}

#[test]
fn phase3_text_prompt_wires_against_legacy() {
    let body = r#"{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
    rt().block_on(async {
        let url = mock_server(body.into());
        let mut client = anthropic("k");
        client.provider.base_url = Some(url);
        let resp = client
            .text()
            .system("be terse")
            .max_tokens(50)
            .prompt("hello")
            .await
            .expect("prompt ok");
        assert_eq!(resp.text, "ok");
    });
}

#[test]
fn phase3_text_submit_batch_returns_handle() {
    let body = r#"{"id":"msgbatch_123"}"#;
    rt().block_on(async {
        let url = mock_server(body.into());
        let mut client = anthropic("k");
        client.provider.base_url = Some(url);
        let handle = client
            .text()
            .system("s")
            .submit_batch(vec!["p1".to_string(), "p2".to_string()])
            .await
            .expect("submit ok");
        assert_eq!(handle.id, "msgbatch_123");
        assert_eq!(handle.provider.api_key, "k");
    });
}

#[test]
fn phase3_upload_run_validation() {
    rt().block_on(async {
        let c = openai("k");

        // Empty: error
        let err = c.upload().run().await.expect_err("empty should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("exactly one of"), "got: {}", msg);

        // Both: error
        let err = openai("k")
            .upload()
            .bytes(vec![1])
            .path("/p")
            .run()
            .await
            .expect_err("both should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("mutually exclusive"), "got: {}", msg);

        // Bytes without filename: error
        let err = openai("k")
            .upload()
            .bytes(vec![1])
            .run()
            .await
            .expect_err("bytes without filename should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("filename"), "got: {}", msg);
    });
}

/// Variant of `mock_server` that captures the raw request bytes into
/// the returned Mutex so the caller can assert on the multipart body.
fn mock_server_capturing(
    response_body: String,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
    let captured_writer = Arc::clone(&captured);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buffer = vec![0u8; 8192];
        let mut total = Vec::new();
        loop {
            let n = stream.read(&mut buffer).unwrap_or(0);
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buffer[..n]);
            if let Some(headers_end) = total.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&total[..headers_end]);
                let cl = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if total.len() >= headers_end + 4 + cl {
                    break;
                }
            }
        }
        *captured_writer.lock().unwrap() = total;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    (format!("http://{}", addr), captured)
}

#[test]
fn phase3_upload_run_bytes_round_trips() {
    let (url, captured) = mock_server_capturing(r#"{"id":"file-zzz"}"#.into());
    rt().block_on(async {
        let mut c = openai("k");
        c.provider.base_url = Some(url);
        let result = c
            .upload()
            .bytes(b"hello".to_vec())
            .filename("note.txt")
            .mime_type("text/plain")
            .run()
            .await
            .expect("bytes upload should succeed");
        assert_eq!(result.id, "file-zzz");
    });

    let body = captured.lock().unwrap().clone();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("filename=\"note.txt\""), "body: {}", body_str);
    assert!(body_str.contains("text/plain"), "body: {}", body_str);
    assert!(body_str.contains("hello"), "body: {}", body_str);
}

#[test]
fn phase3_text_stream_wires_via_callback() {
    // Mock OpenAI-style SSE response.
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"He\"}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\n\
                data: [DONE]\n\n"
        .to_string();
    // Note: mock_server returns Content-Length-based response. SSE in this
    // simplified test body uses Content-Length that matches the entire
    // response (the legacy parser tolerates this).
    rt().block_on(async {
        let url = mock_server(body);
        let mut client = openai("k");
        client.provider.base_url = Some(url);

        let mut chunks: Vec<String> = Vec::new();
        let chunks_ref = &mut chunks;
        let _ = client
            .text()
            .stream("hi", |chunk: &str| {
                chunks_ref.push(chunk.to_string());
            })
            .await;
        assert!(
            !chunks.is_empty(),
            "stream callback should receive at least one chunk"
        );
        assert_eq!(chunks[0], "He");
    });
}

// Stateful Agent reset / state-forking contract tests moved to
// crate-internal `src/builders/internal_tests.rs` — they touch
// `pub(crate)` fields (`bot.state`) and `AgentState::placeholder`.

// === Re-exported types are constructible ===

#[test]
fn type_aliases_constructible() {
    let _: ImageData = ImageData::default();
    let _: MediaRef = MediaRef::default();
    let _: Text = google("k").text();
    let _: Image = google("k").image();
    let _: Agent = google("k").agent();
    let _: Upload = google("k").upload();
}
