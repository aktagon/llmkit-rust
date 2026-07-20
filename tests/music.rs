//
//
//
//
//
//
//
//

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use base64::Engine;
use llmkit::builders::{google, minimax, vertex};
use llmkit::{Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase};
use serde_json::Value;

const LYRIA2_MODEL: &str = "lyria-002";
const LYRIA3_PRO_MODEL: &str = "lyria-3-pro-preview";
const MINIMAX_MODEL: &str = "music-2.6";
const FAKE_WAV: &[u8] = &[b'R', b'I', b'F', b'F', 0x00, b'W', b'A', b'V', b'E'];
const FAKE_MP3: &[u8] = &[0xFF, 0xFB, 0x90, 0x00, b'm', b'p', b'3'];

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
        if let Some(split) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            let header_text = String::from_utf8_lossy(&buffer[..split]).to_string();
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
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

fn engine() -> base64::engine::general_purpose::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

#
async fn generate_music_vertex_predict_round_trips_wav() {
    let encoded = engine().encode(FAKE_WAV);
    let url = serve_once(
        move |captured: Captured| {
            assert!(
                captured
                    .request_line
                    .contains(&format!("{}:predict", LYRIA2_MODEL)),
                "request_line missing model:predict: {}",
                captured.request_line
            );
            //
            assert_eq!(captured.body["instances"][0]["prompt"], "upbeat synthwave");
            assert_eq!(captured.body["parameters"]["sampleCount"], 1);
        },
        serde_json::json!({
            "predictions": [{ "audioContent": encoded, "mimeType": "audio/wav" }]
        }),
    );

    let mut client = vertex("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .music()
        .model(LYRIA2_MODEL)
        .generate("upbeat synthwave")
        .await
        .expect("generate succeeds");

    assert_eq!(resp.audio.len(), 1);
    assert_eq!(resp.audio[0].bytes, FAKE_WAV);
    assert_eq!(resp.audio[0].mime_type, "audio/wav");
}

#
async fn generate_music_google_generate_content_captures_text_and_lyrics() {
    let encoded = engine().encode(FAKE_MP3);
    let url = serve_once(
        move |captured: Captured| {
            assert!(
                captured
                    .request_line
                    .contains(&format!("{}:generateContent", LYRIA3_PRO_MODEL)),
                "request_line missing model:generateContent: {}",
                captured.request_line
            );
            assert!(captured.request_line.contains("key=k"));
            //
            //
            let parts = captured.body["contents"][0]["parts"]
                .as_array()
                .expect("parts array");
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0]["text"], "dream pop");
            assert_eq!(parts[1]["text"], "[verse] neon lights");
            assert_eq!(
                captured.body["generationConfig"]["responseModalities"][0],
                "AUDIO"
            );
        },
        serde_json::json!({
            "candidates": [{
                "content": { "parts": [
                    { "text": "[verse] neon nights" },
                    { "inlineData": { "mimeType": "audio/mpeg", "data": encoded } }
                ]},
                "finishReason": "STOP"
            }]
        }),
    );

    let mut client = google("k");
    client.provider.base_url = Some(url);
    let resp = client
        .music()
        .model(LYRIA3_PRO_MODEL)
        .text("dream pop")
        .lyrics("[verse] neon lights")
        .generate("")
        .await
        .expect("generate succeeds");

    assert_eq!(resp.audio.len(), 1);
    assert_eq!(resp.audio[0].bytes, FAKE_MP3);
    assert_eq!(resp.text, "[verse] neon nights");
    assert_eq!(resp.finish_reason, "STOP");
}

#
async fn generate_music_raw_opt_in_populates_raw() {
    let encoded = engine().encode(FAKE_WAV);
    let url = serve_once(
        |_captured: Captured| {},
        serde_json::json!({
            "predictions": [{ "audioContent": encoded, "mimeType": "audio/wav" }]
        }),
    );
    let mut client = vertex("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .music()
        .model(LYRIA2_MODEL)
        .raw()
        .generate("synthwave")
        .await
        .expect("generate succeeds");
    assert!(resp.raw.is_some());
    assert!(resp.raw.unwrap().get("predictions").is_some());
}

//
//
//
#
async fn generate_music_folds_lyrics_into_prompt_on_instrumental_only_model() {
    let encoded = engine().encode(FAKE_WAV);
    let url = serve_once(
        move |captured: Captured| {
            assert_eq!(
                captured.body["instances"][0]["prompt"],
                "ambient\n[verse] neon lights"
            );
        },
        serde_json::json!({
            "predictions": [{ "audioContent": encoded, "mimeType": "audio/wav" }]
        }),
    );

    let mut client = vertex("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .music()
        .model(LYRIA2_MODEL)
        .lyrics("[verse] neon lights")
        .generate("ambient")
        .await
        .expect("generate succeeds");
    assert_eq!(resp.audio.len(), 1);
}

#
async fn generate_music_requires_model() {
    let result = google("k").music().generate("a song").await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "model"),
        other => panic!("expected model validation error, got {:?}", other),
    }
}

//
//
//
//
//
//

#
async fn generate_music_middleware_fires_pre_then_post() {
    let encoded = engine().encode(FAKE_WAV);
    let url = serve_once(
        |_captured: Captured| {},
        serde_json::json!({
            "predictions": [{ "audioContent": encoded, "mimeType": "audio/wav" }]
        }),
    );

    let calls: Arc<Mutex<Vec<(MiddlewareOp, MiddlewarePhase)>>> = Arc::new(Mutex::new(Vec::new()));
    let observer = calls.clone();
    let mw: MiddlewareFn = Arc::new(move |ev: &Event| {
        observer.lock().unwrap().push((ev.op, ev.phase));
        None
    });

    let mut client = vertex("test-token");
    client.provider.base_url = Some(url);
    client
        .music()
        .model(LYRIA2_MODEL)
        .add_middleware(vec![mw])
        .generate("synthwave")
        .await
        .expect("generate succeeds");

    let recorded = calls.lock().unwrap().clone();
    assert_eq!(recorded.len(), 2);
    assert!(matches!(recorded[0].0, MiddlewareOp::MusicGeneration));
    assert!(matches!(recorded[1].0, MiddlewareOp::MusicGeneration));
    assert_eq!(recorded[0].1, MiddlewarePhase::Pre);
    assert_eq!(recorded[1].1, MiddlewarePhase::Post);
}

#
async fn generate_music_middleware_can_veto() {
    let mw: MiddlewareFn = Arc::new(|ev: &Event| {
        if matches!(ev.phase, MiddlewarePhase::Pre) {
            Some("no music today".into())
        } else {
            None
        }
    });

    let mut client = minimax("mm-key");
    client.provider.base_url = Some("http://127.0.0.1:1".into());
    let result = client
        .music()
        .model(MINIMAX_MODEL)
        .add_middleware(vec![mw])
        .generate("lofi")
        .await;
    match result {
        Err(llmkit::Error::MiddlewareVeto(_)) => {}
        other => panic!("expected MiddlewareVeto error, got {:?}", other),
    }
}
