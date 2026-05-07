use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use std::sync::{Arc, Mutex};

use base64::Engine;
use llmkit::{
    generate_image, Event, ImageOptions, ImageRequest, MiddlewareFn, MiddlewareOp, MiddlewarePhase,
    Part, Provider, ProviderName,
};
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

    let provider = Provider::new(ProviderName::Google, "test-key").with_base_url(url);
    let response = generate_image(
        &provider,
        &ImageRequest {
            prompt: "A nano banana dish".into(),
            model: FLASH_MODEL.into(),
            ..ImageRequest::default()
        },
        &ImageOptions {
            aspect_ratio: Some("16:9".into()),
            image_size: Some("2K".into()),
            include_text: false,
            ..ImageOptions::default()
        },
    )
    .await
    .expect("generate_image succeeds");

    assert_eq!(response.images.len(), 1);
    assert_eq!(response.images[0].mime_type, "image/png");
    assert_eq!(response.images[0].data, FAKE_PNG);
    assert_eq!(response.tokens.input, 12);
    assert_eq!(response.tokens.output, 1290);
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

    let response = generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url(url),
        &ImageRequest {
            prompt: "x".into(),
            model: FLASH_MODEL.into(),
            ..ImageRequest::default()
        },
        &ImageOptions {
            include_text: true,
            ..ImageOptions::default()
        },
    )
    .await
    .expect("generate_image succeeds");

    assert_eq!(response.text, "Here is your image:");
}

#[tokio::test]
async fn generate_image_parts_interleaved_compositional() {
    // ADR-008's motivating scenario: text and reference images interleaved
    // so the model attends to the description-image pairing as intended.
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

    generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url(url),
        &ImageRequest {
            model: FLASH_MODEL.into(),
            parts: vec![
                Part::text("Person:"),
                Part::image("image/png", ref_a),
                Part::text("Outfit:"),
                Part::image("image/png", ref_b),
                Part::text("Generate the person wearing the outfit."),
            ],
            ..ImageRequest::default()
        },
        &ImageOptions::default(),
    )
    .await
    .expect("generate_image succeeds");
}

#[tokio::test]
async fn generate_image_rejects_unsupported_aspect_on_pro() {
    let result = generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url("http://unused".to_string()),
        &ImageRequest {
            prompt: "x".into(),
            model: PRO_MODEL.into(),
            ..ImageRequest::default()
        },
        &ImageOptions {
            aspect_ratio: Some("8:1".into()),
            ..ImageOptions::default()
        },
    )
    .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "aspect_ratio"),
        other => panic!("expected aspect_ratio validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_rejects_512_size_on_pro() {
    let result = generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url("http://unused".to_string()),
        &ImageRequest {
            prompt: "x".into(),
            model: PRO_MODEL.into(),
            ..ImageRequest::default()
        },
        &ImageOptions {
            image_size: Some("512".into()),
            ..ImageOptions::default()
        },
    )
    .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "image_size"),
        other => panic!("expected image_size validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_rejects_too_many_image_parts() {
    let mut too_many = vec![Part::text("describe and edit:")];
    for _ in 0..15 {
        too_many.push(Part::image("image/png", FAKE_PNG.to_vec()));
    }

    let result = generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url("http://unused".to_string()),
        &ImageRequest {
            model: FLASH_MODEL.into(),
            parts: too_many,
            ..ImageRequest::default()
        },
        &ImageOptions::default(),
    )
    .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "parts"),
        other => panic!("expected parts validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_rejects_both_prompt_and_parts() {
    let result = generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url("http://unused".to_string()),
        &ImageRequest {
            model: FLASH_MODEL.into(),
            prompt: "x".into(),
            parts: vec![Part::text("y")],
        },
        &ImageOptions::default(),
    )
    .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "parts"),
        other => panic!("expected parts XOR validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_rejects_both_empty() {
    let result = generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url("http://unused".to_string()),
        &ImageRequest {
            model: FLASH_MODEL.into(),
            ..ImageRequest::default()
        },
        &ImageOptions::default(),
    )
    .await;
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

    generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url(url),
        &ImageRequest {
            prompt: "x".into(),
            model: FLASH_MODEL.into(),
            ..ImageRequest::default()
        },
        &ImageOptions {
            middleware: vec![mw],
            ..ImageOptions::default()
        },
    )
    .await
    .expect("generate_image succeeds");

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

    let result = generate_image(
        &Provider::new(ProviderName::Google, "k").with_base_url("http://unused".to_string()),
        &ImageRequest {
            prompt: "x".into(),
            model: FLASH_MODEL.into(),
            ..ImageRequest::default()
        },
        &ImageOptions {
            middleware: vec![mw],
            ..ImageOptions::default()
        },
    )
    .await;
    match result {
        Err(llmkit::Error::MiddlewareVeto(_)) => {}
        other => panic!("expected MiddlewareVeto error, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_image_requires_model() {
    let result = generate_image(
        &Provider::new(ProviderName::Google, "k"),
        &ImageRequest {
            prompt: "x".into(),
            ..ImageRequest::default()
        },
        &ImageOptions::default(),
    )
    .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "model"),
        other => panic!("expected model validation error, got {:?}", other),
    }
}
