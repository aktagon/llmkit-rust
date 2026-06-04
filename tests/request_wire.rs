// Spike 036 (PIVOT wire-conformance): request-byte conformance, generalized
// across capabilities (structured output, agent-path caching). Asserts the
// OUTBOUND request body is value-equal to the shared golden at
// codegen/testdata/wire/request/v1/<fixture>.json — the SAME golden every SDK
// asserts against. Rust's failure modes: BUG-007 malformed Google body, and
// the agent path could drop caching (BUG-004 class).
//
// ADR-028 governs this suite: one wire test file per SDK, two shared helpers
// (capture + assert), goldens minted only by Go's LLMKIT_UPDATE_WIRE_GOLDEN=1
// path. Mock-server plumbing shared with prompt.rs lives in tests/common/.

mod common;

use common::{serve_once, TestResponse};
use llmkit::builders::{anthropic, google, openai};

fn assert_request_wire_golden(fixture: &str, body: &serde_json::Value) {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    let artifact = repo_root.join(format!("target/wire/request/{fixture}/rust.json"));
    std::fs::create_dir_all(artifact.parent().unwrap()).expect("mkdir artifact dir");
    std::fs::write(&artifact, serde_json::to_string_pretty(body).unwrap()).expect("write artifact");

    let golden_path = repo_root.join(format!(
        "codegen/testdata/wire/request/v1/{fixture}.json"
    ));
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("read golden"))
            .expect("parse golden");
    assert_eq!(
        *body, golden,
        "Rust {fixture} body differs from shared golden"
    );
}

// capture_request_body serves one canned response valid for both text and
// agent paths and returns the outbound JSON the provider received plus the
// raw request text (headers feed the in-driver asserts for load-bearing
// headers, e.g. Anthropic's structured-output beta header).
#[allow(clippy::type_complexity)]
fn capture_request_body() -> (
    String,
    std::sync::Arc<std::sync::Mutex<serde_json::Value>>,
    std::sync::Arc<std::sync::Mutex<String>>,
) {
    let captured = std::sync::Arc::new(std::sync::Mutex::new(serde_json::Value::Null));
    let captured_in = captured.clone();
    let raw_request = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let raw_request_in = raw_request.clone();
    let base_url = serve_once(
        move |request, json| {
            *captured_in.lock().unwrap() = json;
            *raw_request_in.lock().unwrap() = request;
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "id": "msgbatch_test",
                "candidates": [{"content": {"parts": [{"text": "{\"color\":\"blue\"}"}]}}],
                "content": [{"type": "text", "text": "done"}],
                "usage": {"input_tokens": 2000, "output_tokens": 5},
                "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 3}
            })
            .to_string(),
            headers: vec![],
        },
    );
    (base_url, captured, raw_request)
}

// Omits "required" so the goldens witness EnforceStrict normalization
// (auto-required); carries additionalProperties:false so Google's strip is
// witnessed too. See the Go driver comment (the minting reference).
const CANONICAL_STRUCTURED_OUTPUT_SCHEMA: &str = r#"{"type":"object","properties":{"color":{"type":"string"}},"additionalProperties":false}"#;

const CANONICAL_STRUCTURED_OUTPUT_PROMPT: &str = "What color is a clear daytime sky?";

#[tokio::test]
async fn structured_output_wire_google_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .schema(CANONICAL_STRUCTURED_OUTPUT_SCHEMA)
        .prompt(CANONICAL_STRUCTURED_OUTPUT_PROMPT)
        .await
        .expect("structured output prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("structured-output-google", &body);
}

#[tokio::test]
async fn structured_output_wire_openai_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .schema(CANONICAL_STRUCTURED_OUTPUT_SCHEMA)
        .prompt(CANONICAL_STRUCTURED_OUTPUT_PROMPT)
        .await
        .expect("structured output prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("structured-output-openai", &body);
}

#[tokio::test]
async fn structured_output_wire_anthropic_golden() {
    let (base_url, captured, raw_request) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .schema(CANONICAL_STRUCTURED_OUTPUT_SCHEMA)
        .prompt(CANONICAL_STRUCTURED_OUTPUT_PROMPT)
        .await
        .expect("structured output prompt succeeds");

    // ADR-028 Open Questions: load-bearing headers assert in-driver. Without
    // this beta header Anthropic rejects output_format with a 400.
    let request = raw_request.lock().unwrap().to_lowercase();
    assert!(
        request.contains("anthropic-beta: structured-outputs-2025-11-13\r\n"),
        "anthropic-beta header missing from request"
    );

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("structured-output-anthropic", &body);
}

#[tokio::test]
async fn caching_agent_wire_anthropic_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().system("a long stable system prefix").caching();
    bot.prompt("hi").await.expect("agent cached prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("caching-agent-anthropic", &body);
}

#[tokio::test]
async fn caching_text_wire_anthropic_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .system("a long stable system prefix")
        .caching()
        .prompt("hi")
        .await
        .expect("text cached prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("caching-text-anthropic", &body);
}

#[tokio::test]
async fn caching_batch_wire_anthropic_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .system("a long stable system prefix")
        .caching()
        .submit_batch(vec!["hi".to_string()])
        .await
        .expect("batch cached submit succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("caching-batch-anthropic", &body);
}
