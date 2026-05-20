// Typed-builder smoke tests for `c.image().<chain>.generate(...)`.
// Ported from the legacy `generate_image` free function in plan 019.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use std::sync::{Arc, Mutex};

use base64::Engine;
use llmkit::builders::{google, openai};
use llmkit::{Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase};
use std::collections::HashMap;
use serde_json::Value;

const FLASH_MODEL: &str = "gemini-3.1-flash-image-preview";
const PRO_MODEL: &str = "gemini-3-pro-image-preview";
const FAKE_PNG: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

struct Captured {
    request_line: String,
    body: Value,
}

fn serve_once<F>(check: F, response_body: Value) -> String
where
    F: Fn(Captured) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request_text = read_request(&mut stream);
        let split = request_text
            .find("\r\n\r\n")
            .expect("http request separator");
        let request_line = request_text[..split].to_string();
        let body_text = &request_text[split + 4..];
        let body: Value = serde_json::from_str(body_text).unwrap_or(Value::Null);
        check(Captured { request_line, body });

        let response_str = response_body.to_string();
        let response_text = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_str.len(),
            response_str,
        );
        stream
            .write_all(response_text.as_bytes())
            .expect("write response");
    });
    format!("http://{}", addr)
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).expect("read");
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(split) = find_header_end(&buffer) {
            let header_text = String::from_utf8_lossy(&buffer[..split]).to_string();
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or_default())
                })
                .unwrap_or(0);
            let body_start = split + 4;
            while buffer.len() < body_start + content_length {
                let n = stream.read(&mut chunk).expect("read body");
                if n == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..n]);
            }
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Like read_request but preserves raw bytes (no UTF-8 lossy decode), so
/// multipart bodies carrying binary payloads round-trip cleanly.
fn read_request_raw(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).expect("read");
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(split) = find_header_end(&buffer) {
            let header_text = String::from_utf8_lossy(&buffer[..split]).to_string();
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or_default())
                })
                .unwrap_or(0);
            let body_start = split + 4;
            while buffer.len() < body_start + content_length {
                let n = stream.read(&mut chunk).expect("read body");
                if n == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..n]);
            }
            break;
        }
    }
    buffer
}

fn engine() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn flash_response(encoded: &str, prompt_tokens: u32, output_tokens: u32) -> Value {
    serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"inlineData": {"mimeType": "image/png", "data": encoded}}
                ]
            }
        }],
        "usageMetadata": {
            "promptTokenCount": prompt_tokens,
            "candidatesTokenCount": output_tokens,
        }
    })
}

#[tokio::test]
async fn generate_image_google_flash_round_trips_png() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once(
        {
            let encoded = encoded.clone();
            move |captured: Captured| {
                assert!(
                    captured
                        .request_line
                        .contains(&format!("{}:generateContent", FLASH_MODEL)),
                    "request_line missing model: {}",
                    captured.request_line
                );
                assert!(captured.request_line.contains("key=test-key"));
                let mods = captured.body["generationConfig"]["responseModalities"]
                    .as_array()
                    .expect("modalities array");
                assert_eq!(mods.len(), 1);
                assert_eq!(mods[0], "IMAGE");
                assert_eq!(
                    captured.body["generationConfig"]["imageConfig"]["aspectRatio"],
                    "16:9"
                );
                assert_eq!(
                    captured.body["generationConfig"]["imageConfig"]["imageSize"],
                    "2K"
                );
                let _ = encoded; // capture for closure clone
            }
        },
        flash_response(&encoded, 12, 1290),
    );

    let mut client = google("test-key");
    client.provider.base_url = Some(url);
    let response = client
        .image()
        .model(FLASH_MODEL)
        .aspect_ratio("16:9")
        .image_size("2K")
        .generate("A nano banana dish")
        .await
        .expect("generate succeeds");

    assert_eq!(response.images.len(), 1);
    assert_eq!(response.images[0].mime_type, "image/png");
    assert_eq!(response.images[0].bytes, FAKE_PNG);
    assert_eq!(response.usage.input, 12);
    assert_eq!(response.usage.output, 1290);
    assert_eq!(response.text, "");
}

#[tokio::test]
async fn generate_image_with_include_text_captures_text_part() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once(
        |captured: Captured| {
            let mods = captured.body["generationConfig"]["responseModalities"]
                .as_array()
                .expect("modalities array");
            assert_eq!(mods.len(), 2);
            assert_eq!(mods[0], "TEXT");
            assert_eq!(mods[1], "IMAGE");
        },
        serde_json::json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "Here is your image:"},
                        {"inlineData": {"mimeType": "image/png", "data": encoded}}
                    ]
                }
            }],
            "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 100}
        }),
    );

    let mut client = google("k");
    client.provider.base_url = Some(url);
    let response = client
        .image()
        .model(FLASH_MODEL)
        .include_text()
        .generate("x")
        .await
        .expect("generate succeeds");

    assert_eq!(response.text, "Here is your image:");
}

#[tokio::test]
async fn generate_image_parts_interleaved_compositional() {
    // ADR-008's motivating scenario: text and reference images interleaved
    // so the model attends to the description-image pairing as intended.
    // The typed-builder Image's chain methods `text()` and `image()` each
    // append a Part, preserving order — `generate(msg)` then appends `msg`
    // as a final Text Part (when chain has parts) per builders/image.rs.
    let ref_a: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x41];
    let ref_b: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x42];
    let encoded = engine().encode(FAKE_PNG);
    let ref_a_for_check = ref_a.clone();
    let ref_b_for_check = ref_b.clone();
    let url = serve_once(
        move |captured: Captured| {
            let parts = captured.body["contents"][0]["parts"]
                .as_array()
                .expect("parts array");
            assert_eq!(parts.len(), 5);
            assert_eq!(parts[0]["text"], "Person:");
            let decoded_a = base64::engine::general_purpose::STANDARD
                .decode(parts[1]["inlineData"]["data"].as_str().expect("data str"))
                .expect("decode a");
            assert_eq!(decoded_a, ref_a_for_check);
            assert_eq!(parts[2]["text"], "Outfit:");
            let decoded_b = base64::engine::general_purpose::STANDARD
                .decode(parts[3]["inlineData"]["data"].as_str().expect("data str"))
                .expect("decode b");
            assert_eq!(decoded_b, ref_b_for_check);
            assert_eq!(parts[4]["text"], "Generate the person wearing the outfit.");
        },
        flash_response(&encoded, 1, 1),
    );

    let mut client = google("k");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(FLASH_MODEL)
        .text("Person:")
        .image("image/png", ref_a)
        .text("Outfit:")
        .image("image/png", ref_b)
        .generate("Generate the person wearing the outfit.")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_rejects_unsupported_aspect_on_pro() {
    let mut client = google("k");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .image()
        .model(PRO_MODEL)
        .aspect_ratio("8:1")
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "aspect_ratio"),
        other => panic!("expected aspect_ratio validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_rejects_512_size_on_pro() {
    let mut client = google("k");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .image()
        .model(PRO_MODEL)
        .image_size("512")
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "image_size"),
        other => panic!("expected image_size validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_rejects_too_many_image_parts() {
    let mut client = google("k");
    client.provider.base_url = Some("http://unused".to_string());
    let mut chain = client.image().model(FLASH_MODEL).text("describe and edit:");
    for _ in 0..15 {
        chain = chain.image("image/png", FAKE_PNG.to_vec());
    }
    let result = chain.generate("").await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "parts"),
        other => panic!("expected parts validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_rejects_both_empty() {
    // Legacy parallel: both prompt and parts empty rejected with field "prompt".
    // Typed-builder: empty chain + empty msg → ImageRequest with empty
    // prompt and empty parts → same rejection.
    let mut client = google("k");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client.image().model(FLASH_MODEL).generate("").await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "prompt"),
        other => panic!("expected prompt validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_middleware_fires_pre_then_post() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once(|_captured: Captured| {}, flash_response(&encoded, 1, 2));

    let calls: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> = Arc::new(Mutex::new(Vec::new()));
    let calls_for_mw = calls.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        calls_for_mw.lock().unwrap().push((ev.op, ev.phase));
        None
    });

    let mut client = google("k");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(FLASH_MODEL)
        .middleware(vec![mw])
        .generate("x")
        .await
        .expect("generate succeeds");

    let recorded = calls.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert!(matches!(recorded[0].0, MiddlewareOp::ImageGeneration));
    assert!(matches!(recorded[1].0, MiddlewareOp::ImageGeneration));
    assert!(matches!(recorded[0].1, MiddlewarePhase::Pre));
    assert!(matches!(recorded[1].1, MiddlewarePhase::Post));
}

#[tokio::test]
async fn generate_image_middleware_can_veto() {
    let mw: MiddlewareFn = Arc::new(|ev: &Event| {
        if matches!(ev.phase, MiddlewarePhase::Pre) {
            Some("no images today".into())
        } else {
            None
        }
    });

    let mut client = google("k");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .image()
        .model(FLASH_MODEL)
        .middleware(vec![mw])
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::MiddlewareVeto(_)) => {}
        other => panic!("expected MiddlewareVeto error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_requires_model() {
    let result = google("k").image().generate("x").await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "model"),
        other => panic!("expected model validation error, got {:?}", other),
    }
}

// ===== OpenAI Image API (plan 020 phase 4) =====
//
// Two endpoints: /v1/images/generations (JSON; no image parts) and
// /v1/images/edits (multipart/form-data; one or more image parts).
// Output is forced to b64_json so the response shape stays uniform.

const OPENAI_IMAGE_2: &str = "gpt-image-2";

#[derive(Clone)]
struct CapturedRaw {
    request_line: String,
    headers: String,
    body: Vec<u8>,
}

/// Like `serve_once` but exposes the raw body bytes (instead of parsing
/// JSON) so multipart bodies can be inspected.
fn serve_once_raw<F>(check: F, response_body: Value) -> String
where
    F: Fn(CapturedRaw) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let raw = read_request_raw(&mut stream);
        let split = find_subseq(&raw, b"\r\n\r\n").expect("http request separator");
        let header_block = String::from_utf8_lossy(&raw[..split]).to_string();
        let mut iter = header_block.splitn(2, "\r\n");
        let request_line = iter.next().unwrap_or("").to_string();
        let headers = iter.next().unwrap_or("").to_string();
        let body = raw[split + 4..].to_vec();
        check(CapturedRaw {
            request_line,
            headers,
            body,
        });

        let response_str = response_body.to_string();
        let response_text = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_str.len(),
            response_str,
        );
        stream
            .write_all(response_text.as_bytes())
            .expect("write response");
    });
    format!("http://{}", addr)
}

fn openai_image_response(encoded: &str, n: usize) -> Value {
    let data: Vec<Value> = (0..n)
        .map(|_| serde_json::json!({"b64_json": encoded}))
        .collect();
    serde_json::json!({
        "created": 1700000000,
        "data": data,
        "usage": {"input_tokens": 7, "output_tokens": 1500},
    })
}

/// Minimal multipart parser for tests: extracts `(name, filename, content-type, bytes)`
/// per part, in document order. Boundary discovered from the request's
/// Content-Type header. Filename is empty for plain string fields.
struct MultipartPart {
    name: String,
    filename: String,
    mime: String,
    bytes: Vec<u8>,
}

fn parse_multipart(headers: &str, body: &[u8]) -> Vec<MultipartPart> {
    let ctype = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-type:")
                .map(|v| v.trim().to_string())
        })
        .unwrap_or_default();
    let boundary = ctype
        .split(';')
        .find_map(|s| s.trim().strip_prefix("boundary="))
        .map(|s| s.trim_matches('"').to_string())
        .expect("multipart boundary");
    let delim = format!("--{}", boundary);
    let mut parts: Vec<MultipartPart> = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = find_subseq(&body[cursor..], delim.as_bytes()) {
        cursor += rel + delim.len();
        // After boundary: either `--` (terminator) or `\r\n` then headers.
        if cursor + 2 <= body.len() && &body[cursor..cursor + 2] == b"--" {
            break;
        }
        if cursor + 2 <= body.len() && &body[cursor..cursor + 2] == b"\r\n" {
            cursor += 2;
        }
        // Find the part-header / part-body separator.
        let header_end = find_subseq(&body[cursor..], b"\r\n\r\n").expect("part headers");
        let header_text = String::from_utf8_lossy(&body[cursor..cursor + header_end]).to_string();
        cursor += header_end + 4;
        // Find the next boundary to delimit the body.
        let next = find_subseq(&body[cursor..], delim.as_bytes()).expect("next boundary");
        let payload_end = next.saturating_sub(2); // strip trailing \r\n
        let payload = body[cursor..cursor + payload_end].to_vec();
        cursor += next;

        let mut name = String::new();
        let mut filename = String::new();
        let mut mime = String::new();
        for line in header_text.lines() {
            let lower = line.to_ascii_lowercase();
            if let Some(cd) = lower.strip_prefix("content-disposition:") {
                for piece in cd.split(';') {
                    let p = piece.trim();
                    if let Some(v) = p.strip_prefix("name=") {
                        name = v.trim_matches('"').to_string();
                    } else if let Some(v) = p.strip_prefix("filename=") {
                        filename = v.trim_matches('"').to_string();
                    }
                }
            } else if let Some(ct) = lower.strip_prefix("content-type:") {
                mime = ct.trim().to_string();
            }
        }
        parts.push(MultipartPart {
            name,
            filename,
            mime,
            bytes: payload,
        });
    }
    parts
}

fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[tokio::test]
async fn generate_image_openai_generations_omits_response_format() {
    // gpt-image-* always returns b64_json and rejects the
    // response_format parameter — must be absent on the wire.
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            assert!(
                captured.request_line.contains("/v1/images/generations"),
                "wrong path: {}",
                captured.request_line
            );
            let auth = captured
                .headers
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("authorization:"))
                .unwrap_or("");
            assert!(
                auth.contains("Bearer test-key"),
                "missing bearer auth: {}",
                auth
            );
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["model"], OPENAI_IMAGE_2);
            assert_eq!(body["prompt"], "A red circle");
            assert!(
                body.get("response_format").is_none(),
                "response_format must not be set for gpt-image-*; got {:?}",
                body.get("response_format")
            );
            assert!(body.get("size").is_none() || body["size"].is_null());
        },
        openai_image_response(&encoded, 1),
    );

    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(OPENAI_IMAGE_2)
        .generate("A red circle")
        .await
        .expect("generate succeeds");

    assert_eq!(resp.images.len(), 1);
    assert_eq!(resp.images[0].bytes, FAKE_PNG);
    assert_eq!(resp.usage.input, 7);
    assert_eq!(resp.usage.output, 1500);
}

#[tokio::test]
async fn generate_image_openai_edits_single_reference() {
    let encoded = engine().encode(FAKE_PNG);
    let ref_bytes = vec![0x89u8, 0x50, 0x4E, 0x47, 0x41];
    let ref_clone = ref_bytes.clone();
    let url = serve_once_raw(
        move |captured: CapturedRaw| {
            assert!(
                captured.request_line.contains("/v1/images/edits"),
                "wrong path: {}",
                captured.request_line
            );
            let parts = parse_multipart(&captured.headers, &captured.body);
            let model_part = parts.iter().find(|p| p.name == "model").expect("model");
            assert_eq!(model_part.bytes, OPENAI_IMAGE_2.as_bytes());
            let prompt_part = parts.iter().find(|p| p.name == "prompt").expect("prompt");
            assert_eq!(prompt_part.bytes, b"Add a hat");
            let images: Vec<&MultipartPart> =
                parts.iter().filter(|p| p.name == "image[]").collect();
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].bytes, ref_clone);
            assert_eq!(images[0].mime, "image/png");
            assert!(!images[0].filename.is_empty());
        },
        openai_image_response(&encoded, 1),
    );

    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(OPENAI_IMAGE_2)
        .image("image/png", ref_bytes)
        .generate("Add a hat")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.images.len(), 1);
}

#[tokio::test]
async fn generate_image_openai_edits_three_references_preserves_caller_order() {
    let encoded = engine().encode(FAKE_PNG);
    let ref_a = vec![0x89u8, 0x50, 0x41];
    let ref_b = vec![0x89u8, 0x50, 0x42];
    let ref_c = vec![0x89u8, 0x50, 0x43];
    let (a, b, c) = (ref_a.clone(), ref_b.clone(), ref_c.clone());
    let url = serve_once_raw(
        move |captured: CapturedRaw| {
            let parts = parse_multipart(&captured.headers, &captured.body);
            let images: Vec<&MultipartPart> =
                parts.iter().filter(|p| p.name == "image[]").collect();
            assert_eq!(images.len(), 3);
            assert_eq!(images[0].bytes, a);
            assert_eq!(images[1].bytes, b);
            assert_eq!(images[2].bytes, c);
        },
        openai_image_response(&encoded, 1),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .image("image/png", ref_a)
        .image("image/png", ref_b)
        .image("image/png", ref_c)
        .generate("Combine them")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_openai_extra_fields_quality_propagates() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["quality"], "high");
        },
        openai_image_response(&encoded, 1),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    let mut extras = HashMap::new();
    extras.insert("quality".into(), serde_json::json!("high"));
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .extra_fields(extras)
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_openai_extra_fields_n_returns_n_images() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["n"], 4);
        },
        openai_image_response(&encoded, 4),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    let mut extras = HashMap::new();
    extras.insert("n".into(), serde_json::json!(4));
    let resp = client
        .image()
        .model(OPENAI_IMAGE_2)
        .extra_fields(extras)
        .generate("x")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.images.len(), 4);
}

#[tokio::test]
async fn generate_image_openai_arbitrary_size_accepted() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["size"], "1536x1024");
        },
        openai_image_response(&encoded, 1),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .image_size("1536x1024")
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_openai_middleware_fires_pre_then_post() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |_captured: CapturedRaw| {},
        openai_image_response(&encoded, 1),
    );
    let captured: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let observer = captured.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        observer.lock().unwrap().push((ev.op, ev.phase));
        None
    });
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .middleware(vec![mw])
        .generate("x")
        .await
        .expect("generate succeeds");
    let captured = captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 2);
    assert!(matches!(captured[0].0, MiddlewareOp::ImageGeneration));
    assert_eq!(captured[0].1, MiddlewarePhase::Pre);
    assert_eq!(captured[1].1, MiddlewarePhase::Post);
}

#[tokio::test]
async fn generate_image_openai_middleware_can_veto() {
    let mw: MiddlewareFn = Arc::new(|ev: &Event| {
        if matches!(ev.phase, MiddlewarePhase::Pre) {
            Some("blocked".into())
        } else {
            None
        }
    });
    let mut client = openai("test-key");
    // Bind 127.0.0.1:1 would actually fail to connect — but the veto fires
    // before any HTTP attempt, so the URL is irrelevant.
    client.provider.base_url = Some("http://127.0.0.1:1".into());
    let result = client
        .image()
        .model(OPENAI_IMAGE_2)
        .middleware(vec![mw])
        .generate("x")
        .await;
    assert!(matches!(result, Err(llmkit::Error::MiddlewareVeto(_))));
}

// ===== xAI Grok Imagine =====
//
// JSON throughout — both endpoints. Image refs travel as data URLs in the
// body. response_format must be forced to b64_json (xAI defaults to URL).

const GROK_IMAGINE_QUALITY: &str = "grok-imagine-image-quality";

fn grok_image_response(encoded: &str, n: usize, mime: Option<&str>) -> Value {
    let data: Vec<Value> = (0..n)
        .map(|_| {
            let mut entry = serde_json::Map::new();
            entry.insert("b64_json".into(), Value::String(encoded.into()));
            if let Some(m) = mime {
                entry.insert("mime_type".into(), Value::String(m.into()));
            }
            Value::Object(entry)
        })
        .collect();
    serde_json::json!({
        "data": data,
        "usage": {"cost_in_usd_ticks": 1234567},
    })
}

#[tokio::test]
async fn generate_image_grok_generations_forces_b64_json() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            assert!(
                captured.request_line.contains("/v1/images/generations"),
                "wrong path: {}",
                captured.request_line
            );
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["model"], GROK_IMAGINE_QUALITY);
            assert_eq!(body["prompt"], "A red circle");
            // xAI defaults to URL — must be forced to b64_json on the wire.
            assert_eq!(body["response_format"], "b64_json");
            assert!(body.get("image").is_none());
            assert!(body.get("images").is_none());
        },
        grok_image_response(&encoded, 1, Some("image/png")),
    );

    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .generate("A red circle")
        .await
        .expect("generate succeeds");

    assert_eq!(resp.images.len(), 1);
    assert_eq!(resp.images[0].bytes, FAKE_PNG);
    assert_eq!(resp.images[0].mime_type, "image/png");
    // xAI doesn't report token counts; both should be zero rather than fabricated.
    assert_eq!(resp.usage.input, 0);
    assert_eq!(resp.usage.output, 0);
}

#[tokio::test]
async fn generate_image_grok_aspect_ratio_and_resolution() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["aspect_ratio"], "16:9");
            // image_size maps to xAI's `resolution` (different field name from OpenAI's `size`).
            assert_eq!(body["resolution"], "2k");
        },
        grok_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .aspect_ratio("16:9")
        .image_size("2k")
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_grok_rejects_unsupported_aspect_ratio() {
    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .aspect_ratio("4:5")
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "aspect_ratio"),
        other => panic!("expected ValidationError, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_grok_accepts_auto_aspect_ratio() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["aspect_ratio"], "auto");
        },
        grok_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .aspect_ratio("auto")
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_grok_edits_single_reference_as_data_url() {
    let encoded = engine().encode(FAKE_PNG);
    let ref_bytes = vec![0x89u8, 0x50, 0x4E, 0x47, 0x41];
    let expected = format!("data:image/png;base64,{}", engine().encode(&ref_bytes));
    let expected_clone = expected.clone();
    let url = serve_once_raw(
        move |captured: CapturedRaw| {
            assert!(
                captured.request_line.contains("/v1/images/edits"),
                "wrong path: {}",
                captured.request_line
            );
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["image"]["url"], expected_clone);
            assert!(body.get("images").is_none());
        },
        grok_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .image("image/png", ref_bytes)
        .generate("Add a hat")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_grok_edits_three_references_preserves_caller_order() {
    let encoded = engine().encode(FAKE_PNG);
    let ref_a = vec![0x89u8, 0x41];
    let ref_b = vec![0x89u8, 0x42];
    let ref_c = vec![0x89u8, 0x43];
    let url_a = format!("data:image/png;base64,{}", engine().encode(&ref_a));
    let url_b = format!("data:image/png;base64,{}", engine().encode(&ref_b));
    let url_c = format!("data:image/png;base64,{}", engine().encode(&ref_c));
    let url = serve_once_raw(
        move |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            let images = body["images"].as_array().expect("images array");
            assert_eq!(images.len(), 3);
            assert_eq!(images[0]["url"], url_a);
            assert_eq!(images[1]["url"], url_b);
            assert_eq!(images[2]["url"], url_c);
            assert!(body.get("image").is_none());
        },
        grok_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .image("image/png", ref_a)
        .image("image/png", ref_b)
        .image("image/png", ref_c)
        .generate("Combine them")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_grok_extra_fields_n_returns_n_images() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["n"], 4);
        },
        grok_image_response(&encoded, 4, None),
    );
    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some(url);
    let mut extras = HashMap::new();
    extras.insert("n".into(), serde_json::json!(4));
    let resp = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .extra_fields(extras)
        .generate("x")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.images.len(), 4);
}

#[tokio::test]
async fn generate_image_grok_middleware_fires_pre_then_post() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |_captured: CapturedRaw| {},
        grok_image_response(&encoded, 1, None),
    );
    let captured: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> =
        Arc::new(Mutex::new(Vec::new()));
    let observer = captured.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        observer.lock().unwrap().push((ev.op, ev.phase));
        None
    });
    let mut client = llmkit::builders::grok("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .middleware(vec![mw])
        .generate("x")
        .await
        .expect("generate succeeds");
    let captured = captured.lock().unwrap().clone();
    assert_eq!(captured.len(), 2);
    assert!(matches!(captured[0].0, MiddlewareOp::ImageGeneration));
    assert_eq!(captured[0].1, MiddlewarePhase::Pre);
    assert_eq!(captured[1].1, MiddlewarePhase::Post);
}

// =============================================================================
// Plan 020 phase 2 — typed image-gen knob tests
// =============================================================================

#[tokio::test]
async fn generate_image_openai_typed_quality_lands_in_body() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["quality"], "high");
        },
        openai_image_response(&encoded, 1),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .quality("high")
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_openai_typed_output_format_lands_in_body() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["output_format"], "webp");
        },
        openai_image_response(&encoded, 1),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .output_format("webp")
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_openai_typed_background_lands_in_body() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["background"], "transparent");
        },
        openai_image_response(&encoded, 1),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .background("transparent")
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_openai_typed_count_lands_as_n() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["n"], 3);
        },
        openai_image_response(&encoded, 3),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(OPENAI_IMAGE_2)
        .count(3)
        .generate("x")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.images.len(), 3);
}

#[tokio::test]
async fn generate_image_openai_typed_knobs_propagate_as_multipart_fields() {
    let encoded = engine().encode(FAKE_PNG);
    let ref_bytes = vec![0x89u8, 0x50, 0x4E, 0x47];
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let parts = parse_multipart(&captured.headers, &captured.body);
            let q = parts.iter().find(|p| p.name == "quality").expect("quality field");
            assert_eq!(q.bytes, b"medium");
            let f = parts.iter().find(|p| p.name == "output_format").expect("output_format field");
            assert_eq!(f.bytes, b"png");
            let bg = parts.iter().find(|p| p.name == "background").expect("background field");
            assert_eq!(bg.bytes, b"auto");
            let n = parts.iter().find(|p| p.name == "n").expect("n field");
            assert_eq!(n.bytes, b"2");
        },
        openai_image_response(&encoded, 2),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .quality("medium")
        .output_format("png")
        .background("auto")
        .count(2)
        .image("image/png", ref_bytes)
        .generate("edit it")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_google_rejects_quality() {
    let client = google("k");
    let err = client
        .image()
        .model(FLASH_MODEL)
        .quality("high")
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("quality"), "got: {err}");
}

#[tokio::test]
async fn generate_image_google_rejects_output_format() {
    let client = google("k");
    let err = client
        .image()
        .model(FLASH_MODEL)
        .output_format("png")
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("output_format"), "got: {err}");
}

#[tokio::test]
async fn generate_image_google_rejects_background() {
    let client = google("k");
    let err = client
        .image()
        .model(FLASH_MODEL)
        .background("auto")
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("background"), "got: {err}");
}

#[tokio::test]
async fn generate_image_google_rejects_count() {
    let client = google("k");
    let err = client
        .image()
        .model(FLASH_MODEL)
        .count(2)
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("count"), "got: {err}");
}

#[tokio::test]
async fn generate_image_grok_rejects_quality() {
    let client = llmkit::builders::grok("k");
    let err = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .quality("high")
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("quality"), "got: {err}");
}

#[tokio::test]
async fn generate_image_grok_rejects_output_format() {
    let client = llmkit::builders::grok("k");
    let err = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .output_format("png")
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("output_format"), "got: {err}");
}

#[tokio::test]
async fn generate_image_grok_rejects_background() {
    let client = llmkit::builders::grok("k");
    let err = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .background("auto")
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("background"), "got: {err}");
}

#[tokio::test]
async fn generate_image_openai_mask_attaches_to_edit_multipart() {
    let encoded = engine().encode(FAKE_PNG);
    let image_bytes = vec![0x89u8, 0x50, 0x4E, 0x47];
    let mask_bytes = vec![0xDEu8, 0xAD, 0xBE, 0xEF];
    let mask_clone = mask_bytes.clone();
    let url = serve_once_raw(
        move |captured: CapturedRaw| {
            let parts = parse_multipart(&captured.headers, &captured.body);
            let masks: Vec<&MultipartPart> =
                parts.iter().filter(|p| p.name == "mask").collect();
            assert_eq!(masks.len(), 1);
            assert_eq!(masks[0].bytes, mask_clone);
            assert_eq!(masks[0].mime, "image/png");
            assert!(!masks[0].filename.is_empty());
        },
        openai_image_response(&encoded, 1),
    );
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(OPENAI_IMAGE_2)
        .image("image/png", image_bytes)
        .mask("image/png", mask_bytes)
        .generate("patch")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_openai_mask_without_image_parts_rejected() {
    let client = openai("k");
    let err = client
        .image()
        .model(OPENAI_IMAGE_2)
        .mask("image/png", vec![0xDEu8, 0xAD])
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("mask"), "got: {err}");
}

#[tokio::test]
async fn generate_image_google_rejects_mask() {
    let client = google("k");
    let err = client
        .image()
        .model(FLASH_MODEL)
        .mask("image/png", vec![0xDEu8, 0xAD])
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("mask"), "got: {err}");
}

#[tokio::test]
async fn generate_image_grok_rejects_mask() {
    let client = llmkit::builders::grok("k");
    let err = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .mask("image/png", vec![0xDEu8, 0xAD])
        .generate("x")
        .await
        .expect_err("must reject");
    assert!(format!("{err}").contains("mask"), "got: {err}");
}

#[tokio::test]
async fn generate_image_grok_typed_count_lands_as_n() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["n"], 2);
        },
        grok_image_response(&encoded, 2, None),
    );
    let mut client = llmkit::builders::grok("k");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(GROK_IMAGINE_QUALITY)
        .count(2)
        .generate("x")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.images.len(), 2);
}

// =============================================================================
// Vertex Imagen (plan 021) — JSONPredict input mode, bearer auth
// =============================================================================

const VERTEX_IMAGEN_3: &str = "imagen-3.0-generate-002";

fn vertex_image_response(encoded: &str, n: usize, mime: Option<&str>) -> Value {
    let preds: Vec<Value> = (0..n)
        .map(|_| {
            let mut entry = serde_json::Map::new();
            entry.insert("bytesBase64Encoded".into(), Value::String(encoded.into()));
            if let Some(m) = mime {
                entry.insert("mimeType".into(), Value::String(m.into()));
            }
            Value::Object(entry)
        })
        .collect();
    serde_json::json!({"predictions": preds})
}

#[tokio::test]
async fn generate_image_vertex_generations_happy_path() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            assert!(
                captured
                    .request_line
                    .contains(&format!("/{}:predict", VERTEX_IMAGEN_3)),
                "wrong path: {}",
                captured.request_line
            );
            assert!(
                captured.headers.contains("authorization: Bearer test-token")
                    || captured.headers.contains("Authorization: Bearer test-token"),
                "missing bearer auth header: {}",
                captured.headers
            );
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            let instances = body["instances"].as_array().expect("instances array");
            assert_eq!(instances.len(), 1);
            assert_eq!(instances[0]["prompt"], "A red circle");
            assert!(
                instances[0].get("image").is_none(),
                "generation path must not carry instances[0].image"
            );
            assert_eq!(body["parameters"]["sampleCount"], 1);
        },
        vertex_image_response(&encoded, 1, Some("image/png")),
    );

    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(VERTEX_IMAGEN_3)
        .generate("A red circle")
        .await
        .expect("generate succeeds");

    assert_eq!(resp.images.len(), 1);
    assert_eq!(resp.images[0].bytes, FAKE_PNG);
    assert_eq!(resp.images[0].mime_type, "image/png");
    // Vertex predict does not return token counts.
    assert_eq!(resp.usage.input, 0);
    assert_eq!(resp.usage.output, 0);
}

#[tokio::test]
async fn generate_image_vertex_edit_carries_image_on_instance() {
    let encoded = engine().encode(FAKE_PNG);
    let ref_bytes = vec![0x01u8, 0x02, 0x03, 0x04];
    let expected_b64 = engine().encode(&ref_bytes);
    let url = serve_once_raw(
        move |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(
                body["instances"][0]["image"]["bytesBase64Encoded"],
                expected_b64
            );
        },
        vertex_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(VERTEX_IMAGEN_3)
        .image("image/png", ref_bytes)
        .generate("Make it winter")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_vertex_mask_attaches_to_instance() {
    let encoded = engine().encode(FAKE_PNG);
    let mask_bytes = vec![0xAAu8, 0xBB, 0xCC];
    let expected_mask_b64 = engine().encode(&mask_bytes);
    let url = serve_once_raw(
        move |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(
                body["instances"][0]["mask"]["image"]["bytesBase64Encoded"],
                expected_mask_b64
            );
        },
        vertex_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(VERTEX_IMAGEN_3)
        .image("image/png", vec![0x01u8])
        .mask("image/png", mask_bytes)
        .generate("Inpaint here")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_vertex_count_maps_to_sample_count() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["parameters"]["sampleCount"], 4);
        },
        vertex_image_response(&encoded, 4, None),
    );
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(VERTEX_IMAGEN_3)
        .count(4)
        .generate("x")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.images.len(), 4);
}

#[tokio::test]
async fn generate_image_vertex_aspect_ratio_maps_to_parameters() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["parameters"]["aspectRatio"], "16:9");
        },
        vertex_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(VERTEX_IMAGEN_3)
        .aspect_ratio("16:9")
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_vertex_extra_fields_spread_into_parameters() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(body["parameters"]["negativePrompt"], "ugly");
            assert_eq!(body["parameters"]["safetySetting"], "block_some");
        },
        vertex_image_response(&encoded, 1, None),
    );
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(VERTEX_IMAGEN_3)
        .extra_fields({
            let mut m = std::collections::HashMap::new();
            m.insert("negativePrompt".to_string(), Value::String("ugly".to_string()));
            m.insert(
                "safetySetting".to_string(),
                Value::String("block_some".to_string()),
            );
            m
        })
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_vertex_rejects_quality() {
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .image()
        .model(VERTEX_IMAGEN_3)
        .quality("high")
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "quality"),
        other => panic!("expected ValidationError, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_vertex_rejects_output_format() {
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .image()
        .model(VERTEX_IMAGEN_3)
        .output_format("png")
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "output_format"),
        other => panic!("expected ValidationError, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_vertex_rejects_background() {
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some("http://unused".to_string());
    let result = client
        .image()
        .model(VERTEX_IMAGEN_3)
        .background("transparent")
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "background"),
        other => panic!("expected ValidationError, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_google_surfaces_finish_reason_when_blocked() {
    let response = serde_json::json!({
        "candidates": [{
            "finishReason": "IMAGE_OTHER",
            "finishMessage": "Could not generate image. Try rephrasing the prompt.",
        }],
        "usageMetadata": { "promptTokenCount": 8, "candidatesTokenCount": 0 },
    });
    let url = serve_once(|_captured: Captured| {}, response);
    let mut client = google("test-key");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(FLASH_MODEL)
        .generate("blocked")
        .await
        .expect("generate succeeds with no image");
    assert_eq!(resp.images.len(), 0);
    assert_eq!(resp.finish_reason, "IMAGE_OTHER");
    assert_eq!(
        resp.finish_message,
        "Could not generate image. Try rephrasing the prompt."
    );
}

#[tokio::test]
async fn generate_image_google_omits_finish_reason_on_success() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once(|_captured: Captured| {}, flash_response(&encoded, 5, 100));
    let mut client = google("test-key");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(FLASH_MODEL)
        .generate("a cat")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.images.len(), 1);
    assert_eq!(resp.finish_reason, "");
    assert_eq!(resp.finish_message, "");
}

#[tokio::test]
async fn generate_image_vertex_surfaces_rai_filtered_reason() {
    let response = serde_json::json!({
        "predictions": [{ "raiFilteredReason": "Image filtered by safety system" }],
    });
    let url = serve_once_raw(|_captured: CapturedRaw| {}, response);
    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .image()
        .model(VERTEX_IMAGEN_3)
        .generate("blocked")
        .await
        .expect("generate succeeds with no image");
    assert_eq!(resp.images.len(), 0);
    assert_eq!(resp.finish_reason, "Image filtered by safety system");
    assert_eq!(resp.finish_message, "");
}

#[tokio::test]
async fn generate_image_vertex_safety_filter_maps_to_parameters() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            assert_eq!(
                body["parameters"]["safetySetting"],
                llmkit::IMAGE_SAFETY_FILTER_BLOCK_FEW,
                "safetySetting must be in parameters"
            );
        },
        vertex_image_response(&encoded, 1, Some("image/png")),
    );

    let mut client = llmkit::builders::vertex("test-token");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(VERTEX_IMAGEN_3)
        .safety_filter(llmkit::IMAGE_SAFETY_FILTER_BLOCK_FEW)
        .generate("x")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_safety_filter_rejected_on_non_vertex() {
    let result = google("test-key")
        .image()
        .model(FLASH_MODEL)
        .safety_filter("block_few")
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) if field == "safety_filter" => {}
        other => panic!("expected Validation safety_filter error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_google_safety_settings_wire_body() {
    let encoded = engine().encode(FAKE_PNG);
    let url = serve_once_raw(
        |captured: CapturedRaw| {
            let body: Value = serde_json::from_slice(&captured.body).expect("json body");
            let ss = body["safetySettings"].as_array().expect("safetySettings array");
            assert_eq!(ss.len(), 1);
            assert_eq!(ss[0]["category"], llmkit::HARM_CATEGORY_HARASSMENT);
            assert_eq!(ss[0]["threshold"], llmkit::HARM_BLOCK_THRESHOLD_NONE);
        },
        flash_response(&encoded, 1, 1),
    );

    let mut client = llmkit::builders::google("key");
    client.provider.base_url = Some(url);
    client
        .image()
        .model(FLASH_MODEL)
        .safety_settings(vec![llmkit::SafetySetting {
            category: llmkit::HARM_CATEGORY_HARASSMENT.into(),
            threshold: llmkit::HARM_BLOCK_THRESHOLD_NONE.into(),
        }])
        .generate("a cat")
        .await
        .expect("generate succeeds");
}

#[tokio::test]
async fn generate_image_safety_settings_rejected_on_openai() {
    let result = llmkit::builders::openai("key")
        .image()
        .model("gpt-image-1")
        .safety_settings(vec![llmkit::SafetySetting {
            category: llmkit::HARM_CATEGORY_HARASSMENT.into(),
            threshold: llmkit::HARM_BLOCK_THRESHOLD_NONE.into(),
        }])
        .generate("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) if field == "safety_settings" => {}
        other => panic!("expected Validation safety_settings error, got {:?}", other),
    }
}
