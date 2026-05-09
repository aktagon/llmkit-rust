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
use llmkit::{Message, MiddlewareFn, ProviderName, Tool};
use std::sync::Arc;

fn noop_middleware() -> MiddlewareFn {
    Arc::new(|_event| None)
}

#[test]
fn text_chain_lands_in_fields() {
    let mw = noop_middleware();
    let text = google("k")
        .text()
        .caching()
        .file("file-id")
        .history(vec![Message::new("user", "earlier")])
        .image("image/png", vec![0xff])
        .max_tokens(42)
        .middleware(vec![mw])
        .model("text-model")
        .schema(r#"{"type":"object"}"#)
        .system("you are a tutor")
        .temperature(0.7)
        .text("hello");

    assert!(text.caching);
    assert_eq!(text.files.len(), 1);
    assert_eq!(text.files[0].id, "file-id");
    assert_eq!(text.history.len(), 1);
    assert_eq!(text.history[0].content, "earlier");
    assert_eq!(text.max_tokens, Some(42));
    assert_eq!(text.middleware.len(), 1);
    assert_eq!(text.model.as_deref(), Some("text-model"));
    assert_eq!(text.schema.as_deref(), Some(r#"{"type":"object"}"#));
    assert_eq!(text.system.as_deref(), Some("you are a tutor"));
    assert_eq!(text.temperature, Some(0.7));
    assert_eq!(text.parts.len(), 2);
    // Part ordering preserved: image (added first) precedes text.
    match &text.parts[0] {
        llmkit::Part::Image(MediaRef { mime_type, bytes }) => {
            assert_eq!(mime_type, "image/png");
            assert_eq!(bytes, &vec![0xff]);
        }
        _ => panic!("parts[0] not an image"),
    }
    match &text.parts[1] {
        llmkit::Part::Text(s) => assert_eq!(s, "hello"),
        _ => panic!("parts[1] not text"),
    }
}

#[test]
fn image_chain_lands_in_fields() {
    let img = google("k")
        .image()
        .aspect_ratio("16:9")
        .caching()
        .image("image/png", vec![0xff])
        .image_size("2K")
        .include_text()
        .middleware(vec![noop_middleware()])
        .model("img-model")
        .text("compose");

    assert_eq!(img.aspect_ratio.as_deref(), Some("16:9"));
    assert!(img.caching);
    assert_eq!(img.image_size.as_deref(), Some("2K"));
    assert!(img.include_text);
    assert_eq!(img.middleware.len(), 1);
    assert_eq!(img.model.as_deref(), Some("img-model"));
    assert_eq!(img.parts.len(), 2);
}

#[test]
fn agent_chain_lands_in_fields() {
    let tool = Tool::new("calc", "calculator", serde_json::json!({}), |_args| {
        Ok("42".to_string())
    });
    let ag = google("k")
        .agent()
        .caching()
        .max_tokens(1)
        .middleware(vec![noop_middleware()])
        .model("a")
        .system("sys")
        .temperature(0.5)
        .tool(tool);

    assert!(ag.caching);
    assert_eq!(ag.max_tokens, Some(1));
    assert_eq!(ag.middleware.len(), 1);
    assert_eq!(ag.model.as_deref(), Some("a"));
    assert_eq!(ag.system.as_deref(), Some("sys"));
    assert_eq!(ag.temperature, Some(0.5));
    assert_eq!(ag.tools.len(), 1);
    assert_eq!(ag.tools[0].name, "calc");
}

#[test]
fn upload_chain_lands_in_fields() {
    let up = google("k")
        .upload()
        .bytes(b"hi".to_vec())
        .filename("f")
        .middleware(vec![noop_middleware()])
        .mime_type("text/plain")
        .path("/tmp/x");

    assert_eq!(up.bytes, b"hi".to_vec());
    assert_eq!(up.filename.as_deref(), Some("f"));
    assert_eq!(up.middleware.len(), 1);
    assert_eq!(up.mime_type.as_deref(), Some("text/plain"));
    assert_eq!(up.path.as_deref(), Some("/tmp/x"));
}

#[test]
fn client_text_method_returns_fresh_builder_each_call() {
    let c = google("k");
    let a = c.text().system("first");
    let b = c.text().system("second");
    assert_eq!(a.system.as_deref(), Some("first"));
    assert_eq!(b.system.as_deref(), Some("second"));
}

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
    let c = new_client(ProviderName::Openai, "k");
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

        // Bytes-only: deferred
        let err = openai("k")
            .upload()
            .bytes(vec![1])
            .run()
            .await
            .expect_err("bytes-only should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("not yet wired"), "got: {}", msg);
    });
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

// === Stateful Agent ===

#[test]
#[allow(deprecated)] // load-bearing contract test for typed-builder/state semantics; constructs a legacy Agent intentionally
fn phase3_agent_reset_clears_state() {
    use llmkit::Provider;
    let mut bot = anthropic("k").agent().system("s");
    // Manually populate state to simulate post-init.
    let legacy = llmkit::Agent::new(Provider::new(ProviderName::Anthropic, "k"));
    bot.state = Some(llmkit::builders::AgentState::new(legacy));
    bot.reset();
    assert!(bot.state.is_none());
}

#[test]
#[allow(deprecated)] // load-bearing contract test for RUST_BUILDER_POST_MUTATION["Agent"]
fn phase3_agent_state_forking_load_bearing() {
    // Without it, a forked clone via `bot.system("new")` would silently
    // share its parent's history through the same AgentState reference.
    use llmkit::Provider;
    let bot = anthropic("k").agent().system("orig");
    // Manually populate state.
    let legacy = llmkit::Agent::new(Provider::new(ProviderName::Anthropic, "k"));
    let mut bot = bot;
    bot.state = Some(llmkit::builders::AgentState::new(legacy));

    let forked = bot.system("new");
    // Rust's ownership consumed `bot` — we can't check its state any
    // longer (it's been moved into the chain). The contract is on the
    // FORK: chain methods produce a fresh-state clone, so `forked.state`
    // must be None even though we set the parent's state to Some(...).
    assert!(forked.state.is_none());
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
