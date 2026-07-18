// Typed-builder smoke tests for `c.speech().model(..).voice(..).generate(..)`
// (ADR-049). Inworld (SpeechInworld) is reachable via a base_url override and
// tested end-to-end here: the flat-JSON body, Basic auth (key sent verbatim),
// and the base64 audioContent round-trip, plus the pre-flight rejections.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use base64::Engine;
use llmkit::builders::{anthropic, inworld, openai};
use llmkit::Error;
use serde_json::Value;

const INWORLD_TTS2: &str = "inworld-tts-2";
const OPENAI_TTS: &str = "gpt-4o-mini-tts";
const FAKE_WAV: &[u8] = &[b'R', b'I', b'F', b'F', 0x01, b'W', b'A', b'V', b'E'];
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

// Serves a raw (non-JSON) response body, mirroring OpenAI /v1/audio/speech
// which returns the audio bytes directly. Captures the JSON request body.
fn serve_once_raw<F>(check: F, response_bytes: &'static [u8], content_type: &'static str) -> String
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

        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
            content_type,
            response_bytes.len(),
        );
        stream.write_all(header.as_bytes()).expect("write header");
        stream.write_all(response_bytes).expect("write body");
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
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
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

#[tokio::test]
async fn generate_speech_inworld_round_trips_wav() {
    let encoded = engine().encode(FAKE_WAV);
    let url = serve_once(
        move |captured: Captured| {
            assert!(
                captured.request_line.contains("/tts/v1/voice"),
                "request_line missing /tts/v1/voice: {}",
                captured.request_line
            );
            assert!(
                captured.request_line.contains("Basic test-token"),
                "request_line missing Basic auth (key verbatim): {}",
                captured.request_line
            );
            assert_eq!(captured.body["text"], "Hello from llmkit.");
            assert_eq!(captured.body["voiceId"], "Dennis");
            assert_eq!(captured.body["modelId"], INWORLD_TTS2);
            assert_eq!(captured.body["deliveryMode"], "BALANCED");
            assert_eq!(captured.body["audioConfig"]["audioEncoding"], "LINEAR16");
            assert_eq!(captured.body["audioConfig"]["sampleRateHertz"], 22050);
        },
        serde_json::json!({
            "audioContent": encoded,
            "usage": { "processedCharactersCount": 18, "modelId": INWORLD_TTS2 }
        }),
    );

    let mut client = inworld("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .speech()
        .model(INWORLD_TTS2)
        .voice("Dennis")
        .generate("Hello from llmkit.")
        .await
        .expect("generate succeeds");

    assert_eq!(resp.audio.bytes, FAKE_WAV);
    assert_eq!(resp.audio.mime_type, "audio/wav");
}

#[tokio::test]
async fn generate_speech_openai_raw_body_mp3() {
    let url = serve_once_raw(
        move |captured: Captured| {
            assert!(
                captured.request_line.contains("/v1/audio/speech"),
                "request_line missing /v1/audio/speech: {}",
                captured.request_line
            );
            assert!(
                captured.request_line.contains("Bearer test-token"),
                "request_line missing Bearer auth: {}",
                captured.request_line
            );
            assert_eq!(captured.body["model"], OPENAI_TTS);
            assert_eq!(captured.body["input"], "Hello from llmkit.");
            assert_eq!(captured.body["voice"], "alloy");
            assert_eq!(captured.body["response_format"], "mp3");
        },
        FAKE_MP3,
        "audio/mpeg",
    );

    let mut client = openai("test-token");
    client.provider.base_url = Some(url);
    let resp = client
        .speech()
        .model(OPENAI_TTS)
        .voice("alloy")
        .generate("Hello from llmkit.")
        .await
        .expect("generate succeeds");

    assert_eq!(resp.audio.bytes, FAKE_MP3);
    assert_eq!(resp.audio.mime_type, "audio/mpeg");
}

#[tokio::test]
async fn generate_speech_openai_unknown_voice_rejected() {
    let err = openai("test-token")
        .speech()
        .model(OPENAI_TTS)
        .voice("Dennis")
        .generate("Hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation { field: "voice", .. }));
}

#[tokio::test]
async fn generate_speech_unknown_voice_rejected() {
    let err = inworld("test-token")
        .speech()
        .model(INWORLD_TTS2)
        .voice("Nonexistent")
        .generate("Hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation { field: "voice", .. }));
}

#[tokio::test]
async fn generate_speech_unknown_model_rejected() {
    let err = inworld("test-token")
        .speech()
        .model("inworld-tts-99")
        .voice("Dennis")
        .generate("Hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation { field: "model", .. }));
}

#[tokio::test]
async fn generate_speech_missing_voice_rejected() {
    let err = inworld("test-token")
        .speech()
        .model(INWORLD_TTS2)
        .generate("Hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation { field: "voice", .. }));
}

#[tokio::test]
async fn generate_speech_unsupported_provider_rejected() {
    // Anthropic does not support speech generation (OpenAI now does, ADR-051).
    let err = anthropic("test-token")
        .speech()
        .model(INWORLD_TTS2)
        .voice("Dennis")
        .generate("Hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation { field: "provider", .. }));
}

// HANDOFF-036 A5: a 2xx whose body does not parse to audio is a decoding
// error naming the provider and field — never silent empty audio.
#[tokio::test]
async fn speech_missing_audio_content_is_decoding_error() {
    let url = serve_once(
        |_captured: Captured| {},
        serde_json::json!({ "usage": { "processedCharactersCount": 8 } }),
    );

    let mut client = inworld("test-token");
    client.provider.base_url = Some(url);
    let err = client
        .speech()
        .model(INWORLD_TTS2)
        .voice("Dennis")
        .generate("Hello from llmkit.")
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        matches!(err, Error::Unsupported(_)),
        "expected Error::Unsupported, got {err:?}"
    );
    assert!(msg.contains("missing or empty audioContent"), "got: {msg}");
    assert!(msg.contains("Inworld"), "error must name the provider: {msg}");
}

#[tokio::test]
async fn speech_invalid_base64_is_decoding_error() {
    let url = serve_once(
        |_captured: Captured| {},
        serde_json::json!({ "audioContent": "%%not-base64%%" }),
    );

    let mut client = inworld("test-token");
    client.provider.base_url = Some(url);
    let err = client
        .speech()
        .model(INWORLD_TTS2)
        .voice("Dennis")
        .generate("Hello from llmkit.")
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        matches!(err, Error::Unsupported(_)),
        "expected Error::Unsupported, got {err:?}"
    );
    assert!(msg.contains("invalid base64 in audioContent"), "got: {msg}");
}

#[tokio::test]
async fn speech_non_json_2xx_is_decoding_error() {
    let url = serve_once_raw(
        |_captured: Captured| {},
        b"<html>Bad Gateway</html>",
        "text/html",
    );

    let mut client = inworld("test-token");
    client.provider.base_url = Some(url);
    let err = client
        .speech()
        .model(INWORLD_TTS2)
        .voice("Dennis")
        .generate("Hello from llmkit.")
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        matches!(err, Error::Unsupported(_)),
        "expected Error::Unsupported, got {err:?}"
    );
    assert!(msg.contains("not valid JSON"), "got: {msg}");
}
