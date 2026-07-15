// Cross-SDK RESPONSE-body conformance (ADR-065 / prompt 045 Track B) — the Rust
// driver. Sibling of lifecycle_wire.rs. Where the lifecycle suite asserts the
// poll CLASSIFICATION agrees across SDKs, this asserts the body PARSE agrees:
// given the same anchored provider reply, every SDK's public prompt path
// normalizes it to the SAME projection (Usage dims + finish reason + content).
// Go's response_wire_test.go is the FROZEN reference; this driver drops
// target/wire/response/<shape>/rust.json value-equal to the shared golden at
// codegen/testdata/wire/response/v1/<shape>.json. codegen/test_cross_sdk_response.py
// compares.
//
// The parser INPUT is the anchored provider body at
// codegen/testdata/wire/response/v1/bodies/<shape>.json, served verbatim by the
// stdlib-TCP mock (common::serve_sequence). The projection is a serde_json::Value
// (the crate has no serde-derive dependency); the golden emits cost as the float
// literal 0.0 so this Value comparison is float-to-float (Value would treat
// Number(0) != Number(0.0)) — Go/TS/Python normalize 0 <-> 0.0 either way.

mod common;

use common::{serve_sequence, TestExchange, TestResponse};
use llmkit::builders::{anthropic, google, openai, vertex, Client};
use llmkit::{ImageResponse, Response};

fn json_response(body: String) -> TestResponse {
    TestResponse {
        status_line: "HTTP/1.1 200 OK",
        body,
        headers: Vec::new(),
    }
}

// Serves the anchored body verbatim for one request, asserting nothing about it —
// the parse path is single-hop and the parser dispatches on the client's
// provider, not the URL.
fn serve_body(body: String) -> String {
    serve_sequence(vec![TestExchange {
        assert_request: Box::new(|_request, _body| {}),
        response: json_response(body),
    }])
}

// Normalized, cross-SDK-comparable projection — the contract-bearing parse output
// only. cost is forced through f64 so it serializes as 0.0, matching the golden.
fn artifact_from(resp: &Response) -> serde_json::Value {
    serde_json::json!({
        "usage": {
            "input": resp.usage.input,
            "output": resp.usage.output,
            "cacheRead": resp.usage.cache_read,
            "cacheWrite": resp.usage.cache_write,
            "reasoning": resp.usage.reasoning,
            "cost": resp.usage.cost,
        },
        "finishReason": resp.finish_reason,
        "content": resp.text,
        "error": serde_json::Value::Null,
    })
}

// Projection for image responses. Content is the media discriminant
// {kind,mimeType,byteLen,count} (RWR-004) — the four SDKs must agree the same
// body decodes to the same images (the BUG-024 parse-drift class).
fn image_artifact_from(resp: &ImageResponse) -> serde_json::Value {
    let first = resp.images.first();
    serde_json::json!({
        "usage": {
            "input": resp.usage.input,
            "output": resp.usage.output,
            "cacheRead": resp.usage.cache_read,
            "cacheWrite": resp.usage.cache_write,
            "reasoning": resp.usage.reasoning,
            "cost": resp.usage.cost,
        },
        "finishReason": resp.finish_reason,
        "content": {
            "kind": "image",
            "mimeType": first.map(|i| i.mime_type.clone()).unwrap_or_default(),
            "byteLen": first.map(|i| i.bytes.len()).unwrap_or(0),
            "count": resp.images.len(),
        },
        "error": serde_json::Value::Null,
    })
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn read_body(shape: &str) -> String {
    let path = repo_root().join(format!(
        "codegen/testdata/wire/response/v1/bodies/{shape}.json"
    ));
    std::fs::read_to_string(&path).expect("read response body")
}

fn assert_response_golden(shape: &str, resp: &Response) {
    assert_golden(shape, artifact_from(resp));
}

fn assert_golden(shape: &str, artifact: serde_json::Value) {
    let root = repo_root();
    let path = root.join(format!("target/wire/response/{shape}/rust.json"));
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir artifact dir");
    std::fs::write(&path, serde_json::to_string_pretty(&artifact).unwrap())
        .expect("write artifact");

    // Assert value-equality against the shared golden in-driver too, so a drift
    // fails cargo test directly (make check excludes Rust).
    let golden_path = root.join(format!("codegen/testdata/wire/response/v1/{shape}.json"));
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("read golden"))
            .expect("parse golden");
    assert_eq!(
        artifact, golden,
        "Rust response {shape} differs from shared golden"
    );
}

async fn drive(shape: &str, mut client: Client) {
    let url = serve_body(read_body(shape));
    client.provider.base_url = Some(url);
    let resp = client.text().prompt("ping").await.expect("prompt succeeds");
    assert_response_golden(shape, &resp);
}

async fn drive_image(shape: &str, mut client: Client, model: &str) {
    let url = serve_body(read_body(shape));
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(model)
        .generate("a cat")
        .await
        .expect("image generate succeeds");
    assert_golden(shape, image_artifact_from(&resp));
}

#[tokio::test]
async fn response_chat_openai_golden() {
    drive("chat-openai", openai("k")).await;
}

#[tokio::test]
async fn response_chat_anthropic_golden() {
    drive("chat-anthropic", anthropic("k")).await;
}

#[tokio::test]
async fn response_chat_google_golden() {
    drive("chat-google", google("k")).await;
}

// Phase 2: image response dispatch (BUG-024 surface) — one golden per
// llm:imageResponseShape (GoogleParts / DataArrayB64Json / VertexPredictions).
#[tokio::test]
async fn response_image_google_golden() {
    drive_image("image-google", google("k"), "gemini-3.1-flash-image-preview").await;
}

#[tokio::test]
async fn response_image_openai_golden() {
    drive_image("image-openai", openai("k"), "gpt-image-1").await;
}

#[tokio::test]
async fn response_image_vertex_golden() {
    drive_image("image-vertex", vertex("k"), "imagen-3.0-generate-002").await;
}
