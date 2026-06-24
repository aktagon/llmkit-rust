// Typed-builder smoke tests for `c.speech().model(..).voice(..).generate(..)`
// (ADR-049). Inworld (SpeechInworld) is reachable via a base_url override and
// tested end-to-end here: the flat-JSON body, Basic auth (key sent verbatim),
// and the base64 audioContent round-trip, plus the pre-flight rejections.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use base64::Engine;
use llmkit::builders::{inworld, openai};
use llmkit::Error;
use serde_json::Value;

const INWORLD_TTS2: &str = "inworld-tts-2";
const FAKE_WAV: &[u8] = &[b'R', b'I', b'F', b'F', 0x01, b'W', b'A', b'V', b'E'];

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
    let err = openai("test-token")
        .speech()
        .model(INWORLD_TTS2)
        .voice("Dennis")
        .generate("Hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation { field: "provider", .. }));
}
