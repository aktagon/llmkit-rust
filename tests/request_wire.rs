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
            // The inlineData part and the data[] array are the image-shaped
            // fields for the Google and OpenAI image paths (ADR-028
            // two-helper rule: extend the canned response, don't add capture
            // helpers).
            body: serde_json::json!({
                "id": "msgbatch_test",
                "candidates": [{"content": {"parts": [
                    {"text": "{\"color\":\"blue\"}"},
                    {"inlineData": {"mimeType": "image/png", "data": TINY_PNG_BASE64}}
                ]}}],
                "content": [{"type": "text", "text": "done"}],
                "data": [{"b64_json": TINY_PNG_BASE64}],
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

// 69-byte 1x1 RGB PNG (single brick-red pixel) — the FIXED reference image
// for the image-edit fixture. SAME base64 constant in all four SDK drivers.
const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGM4YWQEAALyAS2saifrAAAAAElFTkSuQmCC";

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

// === M2: options fixtures, one per model family (see the Go drivers — the
// minting reference — for WIRE-005 provenance and the live rejection matrix
// that shaped each option chain). ===

#[tokio::test]
async fn options_wire_openai_gpt5_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model("gpt-5")
        .max_tokens(1024)
        .reasoning_effort("low")
        .seed(42)
        .prompt("Summarize the plot of Hamlet in two sentences.")
        .await
        .expect("options gpt-5 prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-openai-gpt5", &body);
}

#[tokio::test]
async fn options_wire_openai_o_series_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model("o4-mini")
        .max_tokens(1024)
        .reasoning_effort("medium")
        .seed(7)
        .prompt("What is the capital of Finland?")
        .await
        .expect("options o4-mini prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-openai-o-series", &body);
}

#[tokio::test]
async fn options_wire_openai_gpt4o_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model("gpt-4o")
        .max_tokens(256)
        .temperature(0.7)
        .top_p(0.9)
        .stop_sequences(vec!["END_OF_LIST".to_string()])
        .seed(42)
        .frequency_penalty(0.25)
        .presence_penalty(0.15)
        .prompt("List three primary colors, then write END_OF_LIST.")
        .await
        .expect("options gpt-4o prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-openai-gpt4o", &body);
}

#[tokio::test]
async fn options_wire_anthropic_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model("claude-sonnet-4-6")
        .max_tokens(2048)
        .thinking_budget(1024)
        .stop_sequences(vec!["END_OF_ANSWER".to_string()])
        .prompt("Explain in one sentence why the sky appears blue at noon, then write END_OF_ANSWER.")
        .await
        .expect("options anthropic prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-anthropic", &body);
}

#[tokio::test]
async fn options_wire_google_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model("gemini-3.5-flash")
        .max_tokens(1024)
        .temperature(0.7)
        .top_p(0.9)
        .top_k(40)
        .stop_sequences(vec!["END_OF_ANSWER".to_string()])
        .seed(7)
        .safety_settings(vec![llmkit::SafetySetting {
            category: "HARM_CATEGORY_DANGEROUS_CONTENT".to_string(),
            threshold: "BLOCK_ONLY_HIGH".to_string(),
        }])
        .prompt("Name the two largest moons of Jupiter, then write END_OF_ANSWER.")
        .await
        .expect("options google prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-google", &body);
}

#[tokio::test]
async fn options_wire_google_gemini25_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model("gemini-2.5-flash")
        .max_tokens(1024)
        .temperature(0.5)
        .thinking_budget(512)
        .prompt("How many planets orbit the Sun? Answer with a number.")
        .await
        .expect("options gemini-2.5 prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-google-gemini25", &body);
}

// === M2: image-generation fixtures (M5 pull-forward, JSON bodies only;
// multipart edits are a WIRE-008 documented exclusion). ===

#[tokio::test]
async fn image_gen_wire_google_flash_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .image()
        .model("gemini-3.1-flash-image-preview")
        .aspect_ratio("16:9")
        .image_size("2K")
        .generate("A lighthouse on a rocky coastline at dusk")
        .await
        .expect("image gen flash succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("image-gen-google-flash", &body);
}

#[tokio::test]
async fn image_gen_wire_google_pro_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .image()
        .model("gemini-3-pro-image-preview")
        .aspect_ratio("4:3")
        .image_size("1K")
        .include_text()
        .generate("A watercolor map of the Baltic Sea")
        .await
        .expect("image gen pro succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("image-gen-google-pro", &body);
}

#[tokio::test]
async fn image_gen_wire_openai_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .image()
        .model("gpt-image-2")
        .image_size("1024x1024")
        .quality("low")
        .output_format("png")
        .background("opaque")
        .count(1)
        .generate("A minimalist line drawing of a sailboat")
        .await
        .expect("image gen openai succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("image-gen-openai", &body);
}

#[tokio::test]
async fn image_edit_wire_google_flash_golden() {
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(TINY_PNG_BASE64)
        .expect("decode tiny PNG constant");
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .image()
        .model("gemini-3.1-flash-image-preview")
        .image("image/png", png)
        .generate("Recolor the square to deep blue")
        .await
        .expect("image edit flash succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("image-edit-google-flash", &body);
}
