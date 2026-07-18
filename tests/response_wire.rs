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
use llmkit::builders::{anthropic, google, inworld, openai, vertex, BatchHandleExt, Client};
use llmkit::models_parsers::{
    parse_anthropic_models_response, parse_google_models_response,
    parse_openai_cohort_models_response, ParseError, ParsedModelsPage,
};
use llmkit::{BatchHandle, ImageResponse, Part, Provider, Response};

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

// Projection for catalogue (/models) responses. Content is the catalogue
// discriminant {kind:"models", count, firstId, lastId, nextCursor, first{...}}
// (ADR-067 Fix B) — the same body must decode to the same model list +
// pagination cursor across all five SDKs. No usage / finishReason: a catalogue is
// not a generation response.
fn models_artifact_from(page: &ParsedModelsPage) -> serde_json::Value {
    let first = page.records.first();
    serde_json::json!({
        "content": {
            "count": page.records.len(),
            "first": {
                "contextWindow": first.map(|r| r.context_window).unwrap_or(0),
                "displayName": first.map(|r| r.display_name.clone()).unwrap_or_default(),
                "maxOutput": first.map(|r| r.max_output).unwrap_or(0),
            },
            "firstId": first.map(|r| r.id.clone()).unwrap_or_default(),
            "kind": "models",
            "lastId": page.records.last().map(|r| r.id.clone()).unwrap_or_default(),
            "nextCursor": page.next_cursor,
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

// Streaming bodies are raw text/event-stream sequences (.sse), not JSON. reqwest
// reads them via Content-Length, so the same single-hop mock serves them.
fn read_stream_body(shape: &str) -> String {
    let path = repo_root().join(format!(
        "codegen/testdata/wire/response/v1/bodies/{shape}.sse"
    ));
    std::fs::read_to_string(&path).expect("read stream body")
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

// B-stream: drive the real streaming path (callback form; Result<Response> is
// Rust's trailing handle) against the SSE mock; the accumulated Response projects
// through the same artifact_from as the sync chat path. Data-only SSE only
// (OpenAI / Google); Anthropic event-typed stream deferred (see PROVENANCE.md).
async fn drive_stream(shape: &str, mut client: Client) {
    let url = serve_body(read_stream_body(shape));
    client.provider.base_url = Some(url);
    let resp = client
        .text()
        .stream("ping", |_chunk: &str| {})
        .await
        .expect("stream succeeds");
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

// SpeechResponse / TranscriptionResponse are not re-exported at the crate root
// (the `structs` module is private), so the projection is built inline on the
// type-inferred value rather than in a typed helper. Content is the media
// discriminant — speech {kind,mimeType,byteLen} (the ADR-018 bytes/mime accessor
// contract); transcript {kind,text,segments}.
async fn drive_speech(shape: &str, mut client: Client, model: &str, voice: &str) {
    let url = serve_body(read_body(shape));
    client.provider.base_url = Some(url);
    let resp = client
        .speech()
        .model(model)
        .voice(voice)
        .generate("hello")
        .await
        .expect("speech generate succeeds");
    let artifact = serde_json::json!({
        "usage": {
            "input": resp.usage.input,
            "output": resp.usage.output,
            "cacheRead": resp.usage.cache_read,
            "cacheWrite": resp.usage.cache_write,
            "reasoning": resp.usage.reasoning,
            "cost": resp.usage.cost,
        },
        "finishReason": "",
        "content": {
            "kind": "speech",
            "mimeType": resp.audio.mime_type,
            "byteLen": resp.audio.bytes.len(),
        },
        "error": serde_json::Value::Null,
    });
    assert_golden(shape, artifact);
}

async fn drive_transcript(shape: &str, mut client: Client, model: &str) {
    let url = serve_body(read_body(shape));
    client.provider.base_url = Some(url);
    let resp = client
        .transcription()
        .model(model)
        .transcribe(vec![Part::audio_bytes("audio/wav", b"RIFF".to_vec())])
        .await
        .expect("transcribe succeeds");
    let artifact = serde_json::json!({
        "usage": {
            "input": resp.usage.input,
            "output": resp.usage.output,
            "cacheRead": resp.usage.cache_read,
            "cacheWrite": resp.usage.cache_write,
            "reasoning": resp.usage.reasoning,
            "cost": resp.usage.cost,
        },
        "finishReason": "",
        "content": {
            "kind": "transcript",
            "text": resp.text,
            "segments": resp.segments.len(),
        },
        "error": serde_json::Value::Null,
    });
    assert_golden(shape, artifact);
}

// Catalogue parse seam is driven DIRECTLY (no HTTP path): feed the anchored
// /models body to the handwritten parser and project the ParsedModelsPage.
fn drive_models(shape: &str, parse: fn(&[u8]) -> Result<ParsedModelsPage, ParseError>) {
    let body = read_body(shape);
    let page = parse(body.as_bytes()).expect("models parse succeeds");
    assert_golden(shape, models_artifact_from(&page));
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

// B-stream: streaming (SSE) response parity — data-only shapes.
#[tokio::test]
async fn response_stream_openai_golden() {
    drive_stream("stream-openai", openai("k")).await;
}

#[tokio::test]
async fn response_stream_google_golden() {
    drive_stream("stream-google", google("k")).await;
}

// Speech (TTS) + transcription (STT) — the media/transcript accessor contract.
#[tokio::test]
async fn response_speech_inworld_golden() {
    drive_speech("speech-inworld", inworld("k"), "inworld-tts-2", "Dennis").await;
}

#[tokio::test]
async fn response_transcription_openai_golden() {
    drive_transcript("transcription-openai", openai("k"), "whisper-1").await;
}

// Catalogue (/models) response parity (ADR-067 Fix B) — one golden per provider
// parse shape (anthropic cursor / openai-cohort / google cursor).
#[test]
fn response_models_anthropic_golden() {
    drive_models("models-anthropic", parse_anthropic_models_response);
}

#[test]
fn response_models_openai_golden() {
    drive_models("models-openai", parse_openai_cohort_models_response);
}

#[test]
fn response_models_google_golden() {
    drive_models("models-google", parse_google_models_response);
}

// Batch results parse (HANDOFF-036 A1): a completed batch's RESULTS file — one
// succeeded line + one errored line (Anthropic result.type=errored carries no
// result.message at the configured result_body_path). Every SDK must SKIP the
// errored line and return the successful subset (count 1); a throwing parser
// would destroy a completed, potentially hours-long batch. Driven through the
// real public path: BatchHandle::poll against a two-hop mock (Anthropic status
// "ended" -> GET .../results serving the anchored JSONL verbatim; the .jsonl
// extension marks a JSONL results file, not a JSON document). Known shared
// assumption (PROVENANCE.md): no SDK matches results by custom_id — all assume
// file line order.
fn batch_results_artifact(responses: &[Response]) -> serde_json::Value {
    let first = match responses.first() {
        Some(r) => serde_json::json!({
            "finishReason": r.finish_reason,
            "text": r.text,
            "usage": {
                "input": r.usage.input,
                "output": r.usage.output,
                "cacheRead": r.usage.cache_read,
                "cacheWrite": r.usage.cache_write,
                "reasoning": r.usage.reasoning,
                "cost": r.usage.cost,
            },
        }),
        None => serde_json::json!({}),
    };
    serde_json::json!({
        "content": {
            "count": responses.len(),
            "first": first,
            "kind": "batch_results",
        },
        "error": serde_json::Value::Null,
    })
}

#[tokio::test]
async fn response_batch_results_anthropic_golden() {
    let results = std::fs::read_to_string(
        repo_root().join("codegen/testdata/wire/response/v1/bodies/batch-results-anthropic.jsonl"),
    )
    .expect("read batch results body");
    let url = serve_sequence(vec![
        TestExchange {
            assert_request: Box::new(|_request, _body| {}),
            response: json_response(
                "{\"id\":\"batch_1\",\"processing_status\":\"ended\"}".to_string(),
            ),
        },
        TestExchange {
            assert_request: Box::new(|_request, _body| {}),
            response: json_response(results),
        },
    ]);

    let mut client = anthropic("test-key");
    client.provider.base_url = Some(url);
    let handle = BatchHandle {
        id: "batch_1".into(),
        provider: Provider {
            name: client.provider.name,
            api_key: client.provider.api_key.clone(),
            model: None,
            base_url: client.provider.base_url.clone(),
            headers: client.provider.headers.clone(),
        },
        raw: false,
    };
    let st = handle.poll().await.expect("poll succeeds");
    let result = st.result.expect("expected a succeeded result");
    assert_golden("batch-results-anthropic", batch_results_artifact(&result));
}
