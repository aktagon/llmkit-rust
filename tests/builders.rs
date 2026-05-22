//! Phase 2b smoke tests for llmkit::builders.
//!
//! Exercises every public symbol — chains, terminal stubs, type
//! aliases, per-provider factories — so the eventual strict Rust
//! coverage gate sees full function coverage on builders.rs.

// Per-provider constructor smoke moved to the generated
// `tests/builders_constructors.rs` — keeps the list and the ontology
// in lockstep.

use llmkit::builders::{
    anthropic, google, openai, Agent, Image, ImageData, MediaRef, Text, Upload,
};

// Field-access tests (text/image/agent/upload chain landings,
// client_text_method_returns_fresh_builder, agent state forking) moved
// to crate-internal `src/builders/internal_tests.rs` when builder
// fields were locked down to `pub(crate)` in plan 020.

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

#[test]
fn text_history_chain_returns_text_builder() {
    // `Text` builder fields are pub(crate) (locked down in plan 020),
    // so assertions on internal state belong in src/builders/internal_tests.rs.
    // This integration smoke test just exercises the chain method
    // through the public surface and confirms it terminates in a
    // usable builder.
    let msgs = vec![
        llmkit::Message {
            role: "user".to_string(),
            content: "earlier turn".to_string(),
            ..Default::default()
        },
        llmkit::Message {
            role: "assistant".to_string(),
            content: "earlier reply".to_string(),
            ..Default::default()
        },
    ];
    let _t: Text = anthropic("k").text().history(msgs).system("be terse");
}

#[test]
fn agent_reset_is_a_no_op_before_first_prompt() {
    // Reset on a never-prompted Agent is a no-op (state is already
    // None). The point of this test is to name `reset` in the public
    // surface — deeper state-forking behavior lives in the crate-
    // internal tests where pub(crate) field access is allowed.
    let mut bot: Agent = anthropic("k").agent().system("be terse").max_tokens(50);
    bot.reset();
    // Calling it twice must still be safe.
    bot.reset();
}

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
