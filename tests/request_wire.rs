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

use common::wire_inputs::*;
use common::{serve_once, TestResponse};
use llmkit::builders::{
    anthropic, bedrock, google, grok, minimax, openai, pixverse, qwen, recraft, together, vertex,
    vidu, workersai, zhipu,
};

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
                "request_id": "vid_test", // VID-007: Grok video-submit handle id
                "task_id": "vid_test", // VideoMinimax: top-level task_id submit handle
                "name": "models/veo-test/operations/op_test", // VideoVeo: operation-name submit handle
                "invocationArn": "arn:test:async-invoke/op_test", // VideoBedrock: invocationArn submit handle
                "output": {"task_id": "vid_test", "task_status": "PENDING"}, // VideoQwen: output.task_id submit handle
                "Resp": {"video_id": 318633193768896i64}, // VideoPixVerse: Resp.video_id submit handle (numeric)
                "candidates": [{"content": {"parts": [
                    {"text": "{\"color\":\"blue\"}"},
                    {"inlineData": {"mimeType": "image/png", "data": WIRE_IMAGE_EDIT_GOOGLE_FLASH_IMAGE_BASE64}}
                ]}}],
                "content": [{"type": "text", "text": "done"}],
                "data": [{"b64_json": WIRE_IMAGE_EDIT_GOOGLE_FLASH_IMAGE_BASE64}],
                "usage": {"input_tokens": 2000, "output_tokens": 5},
                "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 3}
            })
            .to_string(),
            headers: vec![],
        },
    );
    (base_url, captured, raw_request)
}

// Canonical inputs are single-sourced from ontology/wire-fixtures.ttl (plan
// 039) via the generated common/wire_inputs.rs consts. The schema omits
// "required" so the goldens witness EnforceStrict normalization
// (auto-required); it carries additionalProperties:false so Google's strip
// is witnessed too. See the Go driver comment (the minting reference).

#[tokio::test]
async fn structured_output_wire_google_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .schema(WIRE_STRUCTURED_OUTPUT_SCHEMA)
        .prompt(WIRE_STRUCTURED_OUTPUT_PROMPT)
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
        .schema(WIRE_STRUCTURED_OUTPUT_SCHEMA)
        .prompt(WIRE_STRUCTURED_OUTPUT_PROMPT)
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
        .schema(WIRE_STRUCTURED_OUTPUT_SCHEMA)
        .prompt(WIRE_STRUCTURED_OUTPUT_PROMPT)
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
async fn anthropic_text_document_wire_golden() {
    let (base_url, captured, _raw) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model(WIRE_ANTHROPIC_TEXT_DOCUMENT_MODEL)
        .file(WIRE_ANTHROPIC_TEXT_DOCUMENT_FILE_ID)
        .prompt(WIRE_ANTHROPIC_TEXT_DOCUMENT_PROMPT)
        .await
        .expect("anthropic text document prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("anthropic-text-document", &body);
}

#[tokio::test]
async fn openai_text_document_wire_golden() {
    let (base_url, captured, _raw) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model(WIRE_OPENAI_TEXT_DOCUMENT_MODEL)
        .file(WIRE_OPENAI_TEXT_DOCUMENT_FILE_ID)
        .prompt(WIRE_OPENAI_TEXT_DOCUMENT_PROMPT)
        .await
        .expect("openai text document prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("openai-text-document", &body);
}

// === Plan 039: nested-schema fixtures — the recursive normalization walk
// (witness-lint first catch; see the Go drivers for the rationale). ===

#[tokio::test]
async fn structured_output_nested_wire_google_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .schema(WIRE_STRUCTURED_OUTPUT_NESTED_SCHEMA)
        .prompt(WIRE_STRUCTURED_OUTPUT_NESTED_PROMPT)
        .await
        .expect("nested structured output prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("structured-output-nested-google", &body);
}

#[tokio::test]
async fn structured_output_nested_wire_openai_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .schema(WIRE_STRUCTURED_OUTPUT_NESTED_SCHEMA)
        .prompt(WIRE_STRUCTURED_OUTPUT_NESTED_PROMPT)
        .await
        .expect("nested structured output prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("structured-output-nested-openai", &body);
}

#[tokio::test]
async fn structured_output_nested_wire_anthropic_golden() {
    let (base_url, captured, raw_request) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .schema(WIRE_STRUCTURED_OUTPUT_NESTED_SCHEMA)
        .prompt(WIRE_STRUCTURED_OUTPUT_NESTED_PROMPT)
        .await
        .expect("nested structured output prompt succeeds");

    let request = raw_request.lock().unwrap().to_lowercase();
    assert!(
        request.contains("anthropic-beta: structured-outputs-2025-11-13\r\n"),
        "anthropic-beta header missing from request"
    );

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("structured-output-nested-anthropic", &body);
}

#[tokio::test]
async fn caching_agent_wire_anthropic_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().system(WIRE_CACHING_SYSTEM).caching();
    bot.prompt(WIRE_CACHING_PROMPT).await.expect("agent cached prompt succeeds");

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
        .system(WIRE_CACHING_SYSTEM)
        .caching()
        .prompt(WIRE_CACHING_PROMPT)
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
        .system(WIRE_CACHING_SYSTEM)
        .caching()
        .submit_batch(vec![WIRE_CACHING_PROMPT.to_string()])
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
        .model(WIRE_OPTIONS_OPENAI_GPT5_MODEL)
        .max_tokens(WIRE_OPTIONS_OPENAI_GPT5_MAX_TOKENS)
        .reasoning_effort(WIRE_OPTIONS_OPENAI_GPT5_REASONING_EFFORT)
        .seed(WIRE_OPTIONS_OPENAI_GPT5_SEED)
        .prompt(WIRE_OPTIONS_OPENAI_GPT5_PROMPT)
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
        .model(WIRE_OPTIONS_OPENAI_O_SERIES_MODEL)
        .max_tokens(WIRE_OPTIONS_OPENAI_O_SERIES_MAX_TOKENS)
        .reasoning_effort(WIRE_OPTIONS_OPENAI_O_SERIES_REASONING_EFFORT)
        .seed(WIRE_OPTIONS_OPENAI_O_SERIES_SEED)
        .prompt(WIRE_OPTIONS_OPENAI_O_SERIES_PROMPT)
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
        .model(WIRE_OPTIONS_OPENAI_GPT4O_MODEL)
        .max_tokens(WIRE_OPTIONS_OPENAI_GPT4O_MAX_TOKENS)
        .temperature(WIRE_OPTIONS_OPENAI_GPT4O_TEMPERATURE)
        .top_p(WIRE_OPTIONS_OPENAI_GPT4O_TOP_P)
        .stop_sequences(vec![WIRE_OPTIONS_OPENAI_GPT4O_STOP_SEQUENCES.to_string()])
        .seed(WIRE_OPTIONS_OPENAI_GPT4O_SEED)
        .frequency_penalty(WIRE_OPTIONS_OPENAI_GPT4O_FREQUENCY_PENALTY)
        .presence_penalty(WIRE_OPTIONS_OPENAI_GPT4O_PRESENCE_PENALTY)
        .prompt(WIRE_OPTIONS_OPENAI_GPT4O_PROMPT)
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
        .model(WIRE_OPTIONS_ANTHROPIC_MODEL)
        .max_tokens(WIRE_OPTIONS_ANTHROPIC_MAX_TOKENS)
        .thinking_budget(WIRE_OPTIONS_ANTHROPIC_THINKING_BUDGET)
        .stop_sequences(vec![WIRE_OPTIONS_ANTHROPIC_STOP_SEQUENCES.to_string()])
        .prompt(WIRE_OPTIONS_ANTHROPIC_PROMPT)
        .await
        .expect("options anthropic prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-anthropic", &body);
}

#[tokio::test]
async fn options_wire_anthropic_adaptive_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model(WIRE_OPTIONS_ANTHROPIC_ADAPTIVE_MODEL)
        .max_tokens(WIRE_OPTIONS_ANTHROPIC_ADAPTIVE_MAX_TOKENS)
        .reasoning_effort(WIRE_OPTIONS_ANTHROPIC_ADAPTIVE_REASONING_EFFORT)
        .stop_sequences(vec![
            WIRE_OPTIONS_ANTHROPIC_ADAPTIVE_STOP_SEQUENCES.to_string()
        ])
        .prompt(WIRE_OPTIONS_ANTHROPIC_ADAPTIVE_PROMPT)
        .await
        .expect("options anthropic adaptive prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-anthropic-adaptive", &body);
}

#[tokio::test]
async fn options_wire_anthropic_plain_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model(WIRE_OPTIONS_ANTHROPIC_PLAIN_MODEL)
        .max_tokens(WIRE_OPTIONS_ANTHROPIC_PLAIN_MAX_TOKENS)
        .temperature(WIRE_OPTIONS_ANTHROPIC_PLAIN_TEMPERATURE)
        .top_k(WIRE_OPTIONS_ANTHROPIC_PLAIN_TOP_K)
        .stop_sequences(vec![WIRE_OPTIONS_ANTHROPIC_PLAIN_STOP_SEQUENCES.to_string()])
        .prompt(WIRE_OPTIONS_ANTHROPIC_PLAIN_PROMPT)
        .await
        .expect("options anthropic plain prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("options-anthropic-plain", &body);
}

#[tokio::test]
async fn options_wire_google_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model(WIRE_OPTIONS_GOOGLE_MODEL)
        .max_tokens(WIRE_OPTIONS_GOOGLE_MAX_TOKENS)
        .temperature(WIRE_OPTIONS_GOOGLE_TEMPERATURE)
        .top_p(WIRE_OPTIONS_GOOGLE_TOP_P)
        .top_k(WIRE_OPTIONS_GOOGLE_TOP_K)
        .stop_sequences(vec![WIRE_OPTIONS_GOOGLE_STOP_SEQUENCES.to_string()])
        .seed(WIRE_OPTIONS_GOOGLE_SEED)
        .reasoning_effort(WIRE_OPTIONS_GOOGLE_REASONING_EFFORT)
        .safety_settings(vec![llmkit::SafetySetting {
            category: WIRE_OPTIONS_GOOGLE_SAFETY_CATEGORY.to_string(),
            threshold: WIRE_OPTIONS_GOOGLE_SAFETY_THRESHOLD.to_string(),
        }])
        .prompt(WIRE_OPTIONS_GOOGLE_PROMPT)
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
        .model(WIRE_OPTIONS_GOOGLE_GEMINI25_MODEL)
        .max_tokens(WIRE_OPTIONS_GOOGLE_GEMINI25_MAX_TOKENS)
        .temperature(WIRE_OPTIONS_GOOGLE_GEMINI25_TEMPERATURE)
        .thinking_budget(WIRE_OPTIONS_GOOGLE_GEMINI25_THINKING_BUDGET)
        .prompt(WIRE_OPTIONS_GOOGLE_GEMINI25_PROMPT)
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
        .model(WIRE_IMAGE_GEN_GOOGLE_FLASH_MODEL)
        .aspect_ratio(WIRE_IMAGE_GEN_GOOGLE_FLASH_ASPECT_RATIO)
        .image_size(WIRE_IMAGE_GEN_GOOGLE_FLASH_IMAGE_SIZE)
        .generate(WIRE_IMAGE_GEN_GOOGLE_FLASH_PROMPT)
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
        .model(WIRE_IMAGE_GEN_GOOGLE_PRO_MODEL)
        .aspect_ratio(WIRE_IMAGE_GEN_GOOGLE_PRO_ASPECT_RATIO)
        .image_size(WIRE_IMAGE_GEN_GOOGLE_PRO_IMAGE_SIZE)
        .include_text()
        .generate(WIRE_IMAGE_GEN_GOOGLE_PRO_PROMPT)
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
        .model(WIRE_IMAGE_GEN_OPENAI_MODEL)
        .image_size(WIRE_IMAGE_GEN_OPENAI_IMAGE_SIZE)
        .quality(WIRE_IMAGE_GEN_OPENAI_QUALITY)
        .output_format(WIRE_IMAGE_GEN_OPENAI_OUTPUT_FORMAT)
        .background(WIRE_IMAGE_GEN_OPENAI_BACKGROUND)
        .count(WIRE_IMAGE_GEN_OPENAI_COUNT)
        .generate(WIRE_IMAGE_GEN_OPENAI_PROMPT)
        .await
        .expect("image gen openai succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("image-gen-openai", &body);
}

// Recraft generations JSON body (JSONGenerations shape): {model, prompt,
// size, n} plus the forced response_format=b64_json (Recraft defaults to URL
// delivery; the SDK forces b64_json for a uniform decode path).
#[tokio::test]
async fn image_gen_wire_recraft_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = recraft("key");
    client.provider.base_url = Some(base_url);
    client
        .image()
        .model(WIRE_IMAGE_GEN_RECRAFT_MODEL)
        .image_size(WIRE_IMAGE_GEN_RECRAFT_IMAGE_SIZE)
        .count(WIRE_IMAGE_GEN_RECRAFT_COUNT)
        .generate(WIRE_IMAGE_GEN_RECRAFT_PROMPT)
        .await
        .expect("image gen recraft succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("image-gen-recraft", &body);
}

#[tokio::test]
async fn image_edit_wire_google_flash_golden() {
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(WIRE_IMAGE_EDIT_GOOGLE_FLASH_IMAGE_BASE64)
        .expect("decode tiny PNG constant");
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .image()
        .model(WIRE_IMAGE_EDIT_GOOGLE_FLASH_MODEL)
        .image(WIRE_IMAGE_EDIT_GOOGLE_FLASH_IMAGE_MIME, png)
        .generate(WIRE_IMAGE_EDIT_GOOGLE_FLASH_PROMPT)
        .await
        .expect("image edit flash succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("image-edit-google-flash", &body);
}

// ADR-034 / VID-007: Grok video-submit body {model, prompt}. serve_once
// answers the single submit POST with a request_id so submit returns a
// VideoHandle (discarded — only the outbound submit bytes are asserted).
#[tokio::test]
async fn video_grok_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = grok("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_GROK_MODEL)
        .submit(WIRE_VIDEO_GROK_PROMPT)
        .await
        .expect("video submit grok succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-grok", &body);
}

// BUG-010: Grok image-to-video submit body {model, prompt, image:{url}}. The
// seed frame inlines as a data URL at image.url (the Grok image-EDIT
// encoding); the text-to-video golden above has no image field.
#[tokio::test]
async fn video_grok_i2v_wire_golden() {
    use base64::Engine;
    let seed = base64::engine::general_purpose::STANDARD
        .decode(WIRE_VIDEO_GROK_I2V_IMAGE_BASE64)
        .expect("decode tiny PNG constant");
    let (base_url, captured, _) = capture_request_body();
    let mut client = grok("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_GROK_I2V_MODEL)
        .image(WIRE_VIDEO_GROK_I2V_IMAGE_MIME, seed)
        .submit(WIRE_VIDEO_GROK_I2V_PROMPT)
        .await
        .expect("video i2v submit grok succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-grok-i2v", &body);
}

// ADR-034 fan-out: Zhipu CogVideoX video-submit body {model, prompt} —
// structurally identical to Grok's (the shared {model, prompt} arm); the
// lifecycle divergence is delivery-side, covered by the unit tests.
#[tokio::test]
async fn video_zhipu_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = zhipu("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_ZHIPU_MODEL)
        .submit(WIRE_VIDEO_ZHIPU_PROMPT)
        .await
        .expect("video submit zhipu succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-zhipu", &body);
}

// ADR-034 fan-out: Vidu (Shengshu) video-submit body {model, prompt} —
// structurally identical to Grok's/Zhipu's (the shared {model, prompt} arm);
// the lifecycle divergence is delivery-side, covered by the unit tests.
#[tokio::test]
async fn video_vidu_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = vidu("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_VIDU_MODEL)
        .submit(WIRE_VIDEO_VIDU_PROMPT)
        .await
        .expect("video submit vidu succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-vidu", &body);
}

// ADR-034 fan-out: PixVerse video-submit body {model, prompt, duration,
// quality, aspect_ratio} — the dedicated PixVerse arm (all five fields
// required); the dynamic Ai-trace-id header is omitted from the golden (it is
// a per-request UUID) and asserted in the lifecycle unit tests.
#[tokio::test]
async fn video_pixverse_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = pixverse("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_PIXVERSE_MODEL)
        .submit(WIRE_VIDEO_PIXVERSE_PROMPT)
        .await
        .expect("video submit pixverse succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-pixverse", &body);
}

// ADR-034 fan-out: Together video-submit body {model, prompt} — structurally
// identical to Grok's/Zhipu's (the shared {model, prompt} arm); the lifecycle
// divergence is delivery-side, covered by the unit tests.
#[tokio::test]
async fn video_together_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = together("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_TOGETHER_MODEL)
        .submit(WIRE_VIDEO_TOGETHER_PROMPT)
        .await
        .expect("video submit together succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-together", &body);
}

// ADR-034 fan-out: Qwen (DashScope) video-submit body is the NESTED
// {model, input:{prompt}} shape — the first divergent submit body. Also
// asserts the load-bearing X-DashScope-Async: enable header in-driver (mirrors
// the Anthropic beta-header assert; the raw request string carries the header).
#[tokio::test]
async fn video_qwen_wire_golden() {
    let (base_url, captured, raw_request) = capture_request_body();
    let mut client = qwen("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_QWEN_MODEL)
        .submit(WIRE_VIDEO_QWEN_PROMPT)
        .await
        .expect("video submit qwen succeeds");

    let request = raw_request.lock().unwrap().to_lowercase();
    assert!(
        request.contains("x-dashscope-async: enable\r\n"),
        "X-DashScope-Async: enable header missing from request"
    );

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-qwen", &body);
}

// ADR-034 fan-out: MiniMax video-submit body is the shared {model, prompt}.
// The two-hop result (poll file_id -> file-retrieve download_url) is
// delivery-side, covered by the unit tests.
#[tokio::test]
async fn video_minimax_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = minimax("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_MINIMAX_MODEL)
        .submit(WIRE_VIDEO_MINIMAX_PROMPT)
        .await
        .expect("video submit minimax succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-minimax", &body);
}

// ADR-034 fan-out: Google Veo video-submit body is the nested
// {instances:[{prompt}]} shape — the first video-submit body with NO model
// field, because Veo carries the model in the submit PATH
// (/v1beta/models/{model}:predictLongRunning). The LRO lifecycle and ?key=
// query-param auth are delivery-side, covered by the unit tests.
#[tokio::test]
async fn video_veo_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_GOOGLE_MODEL)
        .submit(WIRE_VIDEO_GOOGLE_PROMPT)
        .await
        .expect("video submit veo succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-google", &body);
}

// ADR-034 fan-out: AWS Bedrock Nova Reel video-submit body — the model is
// carried in the BODY (modelId), the prompt nests under
// modelInput.textToVideoParams.text, and the caller S3 URI lands under
// outputDataConfig.s3OutputDataConfig.s3Uri. The submit is SigV4-signed; the
// mock captures the outbound body regardless of the (keyless) signature.
#[tokio::test]
async fn video_bedrock_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = bedrock("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_BEDROCK_MODEL)
        .output_uri("s3://llmkit-wire-fixtures/out/")
        .submit(WIRE_VIDEO_BEDROCK_PROMPT)
        .await
        .expect("video submit bedrock succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-bedrock", &body);
}

// ADR-034 delivery-mode phase: Vertex Veo video-submit body is the nested
// {instances:[{prompt}]} shape — byte-identical to the Veo golden (model in the
// PATH, not the body). The POST-poll lifecycle (:fetchPredictOperation,
// inline-base64 download delivery) is delivery-side, covered by the unit tests.
#[tokio::test]
async fn video_vertex_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = vertex("key");
    client.provider.base_url = Some(base_url);
    client
        .video()
        .model(WIRE_VIDEO_VERTEX_MODEL)
        .submit(WIRE_VIDEO_VERTEX_PROMPT)
        .await
        .expect("video submit vertex succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("video-vertex", &body);
}

// Prompt 043: Cloudflare Workers AI's OpenAI-compatible chat-completions body
// {model, messages, max_tokens, temperature, top_p} — structurally identical to
// the gpt-4o options golden (OpenAI ArgsFormat, system-in-messages); the novel
// bit (account-id-in-URL) is delivery-side, not request-body-side.
#[tokio::test]
async fn workersai_wire_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = workersai("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .model(WIRE_WORKERSAI_MODEL)
        .max_tokens(WIRE_WORKERSAI_MAX_TOKENS)
        .temperature(WIRE_WORKERSAI_TEMPERATURE)
        .top_p(WIRE_WORKERSAI_TOP_P)
        .prompt(WIRE_WORKERSAI_PROMPT)
        .await
        .expect("workersai prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("workersai", &body);
}

// === TASK-002: tool-definition fixtures across the four chat wire families ===
//
// One tool is registered on an agent and the agent is prompted once: the mock
// returns a plain text response, so the agent loop sends exactly ONE request
// (carrying the tool defs) and terminates. The driver asserts that request's
// body byte-for-byte against the per-family golden — pinning each family's
// tool-definition wire block byte-identically across all four SDKs. NOT
// live-anchored — parity held by the cross-SDK comparator + mock body, like
// the keyless providers. See the Go drivers (the minting reference).

// The Bedrock chat/agent path validates AWS_REGION and SigV4-signs the request
// before sending, so the keyless drivers must seed dummy AWS env vars (mirrors
// prompt.rs::prompt_bedrock_sigv4_shape). The lock serializes the env mutation
// across the two Bedrock tests in this file's binary.
fn aws_env_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn set_bedrock_env() {
    std::env::set_var("AWS_REGION", "us-east-1");
    std::env::set_var("AWS_SECRET_ACCESS_KEY", "SECRET");
    std::env::set_var("AWS_SESSION_TOKEN", "SESSION");
}

// wire_tool_def builds the single canonical tool from the generated wire-input
// consts (ontology/wire-fixtures.ttl single source). The run closure is never
// invoked: the mock returns plain text, so the agent loop makes one request.
fn wire_tool_def() -> llmkit::Tool {
    let schema: serde_json::Value =
        serde_json::from_str(WIRE_TOOL_TOOL_SCHEMA).expect("parse tool schema");
    llmkit::Tool::new(
        WIRE_TOOL_TOOL_NAME,
        WIRE_TOOL_TOOL_DESCRIPTION,
        schema,
        |_args| Ok(String::new()),
    )
}

#[tokio::test]
async fn tooldef_wire_openai_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().add_tool(wire_tool_def());
    bot.prompt(WIRE_TOOL_PROMPT)
        .await
        .expect("openai tool-def prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("tooldef-openai", &body);
}

#[tokio::test]
async fn tooldef_wire_anthropic_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = anthropic("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().add_tool(wire_tool_def());
    bot.prompt(WIRE_TOOL_PROMPT)
        .await
        .expect("anthropic tool-def prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("tooldef-anthropic", &body);
}

#[tokio::test]
async fn tooldef_wire_google_golden() {
    let (base_url, captured, _) = capture_request_body();
    let mut client = google("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().add_tool(wire_tool_def());
    bot.prompt(WIRE_TOOL_PROMPT)
        .await
        .expect("google tool-def prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("tooldef-google", &body);
}

#[tokio::test]
async fn tooldef_wire_bedrock_golden() {
    let _guard = aws_env_lock().lock().expect("lock");
    set_bedrock_env();
    let (base_url, captured, _) = capture_request_body();
    let mut client = bedrock("key");
    client.provider.base_url = Some(base_url);
    let mut bot = client.agent().add_tool(wire_tool_def());
    bot.prompt(WIRE_TOOL_PROMPT)
        .await
        .expect("bedrock tool-def prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("tooldef-bedrock", &body);
}

// TASK-002: the Bedrock Converse message body with NO tools — the ChatBedrock
// message-transform arm that had no chat golden before TASK-002 (only
// video-bedrock existed). Also witnesses Bedrock's full chat option surface
// (Temperature/TopP/MaxTokens/StopSequences -> inferenceConfig).
#[tokio::test]
async fn bedrock_chat_wire_golden() {
    let _guard = aws_env_lock().lock().expect("lock");
    set_bedrock_env();
    let (base_url, captured, _) = capture_request_body();
    let mut client = bedrock("key");
    client.provider.base_url = Some(base_url);
    client
        .text()
        .max_tokens(WIRE_BEDROCK_CHAT_MAX_TOKENS)
        .temperature(WIRE_BEDROCK_CHAT_TEMPERATURE)
        .top_p(WIRE_BEDROCK_CHAT_TOP_P)
        .stop_sequences(vec![WIRE_BEDROCK_CHAT_STOP_SEQUENCES.to_string()])
        .prompt(WIRE_BEDROCK_CHAT_PROMPT)
        .await
        .expect("bedrock chat prompt succeeds");

    let body = captured.lock().unwrap().clone();
    assert_request_wire_golden("bedrock-chat", &body);
}
