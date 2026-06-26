// Typed-builder smoke tests for `c.transcription().submit(...)` + the
// TranscriptionHandle::wait extension trait (ADR-048). Mirror of
// go/transcription_test.go.
//
// AssemblyAI (TranscriptionAssemblyAI) is reachable via a base_url override
// and tested end-to-end: an optional upload hop, a POST {audio_url} submit,
// then polled GETs (processing -> completed). The shared serve_sequence helper
// serves each request on its own connection, so the poll loop talks to the
// same mock across the sequence.
//
// wait() uses the default poll cadence (3s); the processing-poll test serves
// `completed` on the first poll so the loop returns without a sleep. A single
// dedicated test exercises the processing->completed transition (one 3s sleep).

mod common;

use common::{serve_sequence, TestExchange, TestResponse};
use llmkit::builders::{anthropic, assemblyai, openai};
use llmkit::builders::TranscriptionHandleExt;
use llmkit::Part;

const ASSEMBLYAI_AUDIO_URL: &str = "https://storage.example.com/meeting-2026-06-24.mp3";

fn json_response(body: serde_json::Value) -> TestResponse {
    TestResponse {
        status_line: "HTTP/1.1 200 OK",
        body: body.to_string(),
        headers: Vec::new(),
    }
}

// completedTranscript is the AssemblyAI transcript object on terminal success:
// the full text plus word-level timing (start/end in milliseconds), with a
// diarized speaker label on the first word only.
fn completed_transcript() -> serde_json::Value {
    serde_json::json!({
        "id": "transcript-7c2",
        "status": "completed",
        "text": "The quarterly review is scheduled for Tuesday.",
        "words": [
            { "text": "The", "start": 120, "end": 280, "speaker": "A" },
            { "text": "quarterly", "start": 280, "end": 760 },
            { "text": "review", "start": 760, "end": 1100 },
        ],
    })
}

// submit_exchange asserts the submit POST carries the raw key (no Bearer
// prefix) and the {audio_url} body, then returns the queued handle.
fn submit_exchange(expected_audio_url: &'static str) -> TestExchange {
    TestExchange {
        assert_request: Box::new(move |request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /v2/transcript"),
                "submit must POST the transcript endpoint: {request}"
            );
            // AssemblyAI auth: the raw key with no Bearer prefix (HeaderAPIKey).
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: test-key"),
                "submit must carry the raw key (no Bearer prefix): {request}"
            );
            assert!(
                !request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer"),
                "submit must NOT use a Bearer prefix: {request}"
            );
            assert_eq!(body["audio_url"], expected_audio_url);
        }),
        response: json_response(serde_json::json!({
            "id": "transcript-7c2",
            "status": "queued",
        })),
    }
}

fn poll_exchange(body: serde_json::Value) -> TestExchange {
    TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /v2/transcript/transcript-7c2"),
                "poll must GET the per-id endpoint: {request}"
            );
        }),
        response: json_response(body),
    }
}

#[tokio::test]
async fn submit_and_wait_assemblyai_text_and_segments() {
    let exchanges = vec![
        submit_exchange(ASSEMBLYAI_AUDIO_URL),
        poll_exchange(completed_transcript()),
    ];
    let url = serve_sequence(exchanges);

    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);

    let handle = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "transcript-7c2");

    let resp = handle.wait().await.expect("wait succeeds");
    assert_eq!(resp.text, "The quarterly review is scheduled for Tuesday.");
    assert_eq!(resp.segments.len(), 3);
    assert_eq!(resp.segments[0].text, "The");
    assert_eq!(resp.segments[0].start, 120);
    assert_eq!(resp.segments[0].end, 280);
    assert_eq!(resp.segments[0].speaker, "A");
    assert_eq!(resp.segments[1].speaker, "");
    assert_eq!(resp.usage.input, 0);
}

#[tokio::test]
async fn submit_and_wait_assemblyai_processing_then_completed() {
    // submit -> poll(processing) -> poll(completed). One processing poll
    // incurs a single default-cadence (3s) sleep — acceptable for one test.
    let exchanges = vec![
        submit_exchange(ASSEMBLYAI_AUDIO_URL),
        poll_exchange(serde_json::json!({ "id": "transcript-7c2", "status": "processing" })),
        poll_exchange(completed_transcript()),
    ];
    let url = serve_sequence(exchanges);

    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);

    let handle = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect("submit succeeds");
    let resp = handle.wait().await.expect("wait succeeds");
    assert_eq!(resp.text, "The quarterly review is scheduled for Tuesday.");
    assert_eq!(resp.segments.len(), 3);
}

#[tokio::test]
async fn audio_bytes_upload_hop() {
    const UPLOADED_URL: &str = "https://cdn.assemblyai.com/upload/abc123";
    let wav = b"RIFF....WAVEfmt fake-audio-bytes".to_vec();
    let wav_len = wav.len();

    let upload_exchange = TestExchange {
        assert_request: Box::new(move |request: String, _body| {
            assert!(
                request.contains("POST /v2/upload"),
                "upload must POST the upload endpoint: {request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("content-type: application/octet-stream"),
                "upload must send raw octet-stream: {request}"
            );
            // The raw audio bytes ride the request body (not JSON).
            let split = request.find("\r\n\r\n").expect("body present");
            let body = &request[split + 4..];
            assert_eq!(body.len(), wav_len, "upload body must carry the raw bytes");
        }),
        response: json_response(serde_json::json!({ "upload_url": UPLOADED_URL })),
    };

    let exchanges = vec![
        upload_exchange,
        submit_exchange(UPLOADED_URL),
        poll_exchange(completed_transcript()),
    ];
    let url = serve_sequence(exchanges);

    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);

    let handle = client
        .transcription()
        .submit(vec![Part::audio_bytes("audio/wav", wav)])
        .await
        .expect("submit succeeds");
    let resp = handle.wait().await.expect("wait succeeds");
    assert_eq!(resp.text, "The quarterly review is scheduled for Tuesday.");
}

#[tokio::test]
async fn error_status_surfaces_as_error() {
    let failed = serde_json::json!({
        "id": "transcript-7c2",
        "status": "error",
        "error": format!(
            "Download error, unable to download {ASSEMBLYAI_AUDIO_URL}"
        ),
    });
    let exchanges = vec![submit_exchange(ASSEMBLYAI_AUDIO_URL), poll_exchange(failed)];
    let url = serve_sequence(exchanges);

    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);

    let handle = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect("submit succeeds");
    let err = handle.wait().await.expect_err("error status must fail");
    assert!(
        err.to_string().contains("Download error"),
        "error must carry the provider message: {err}"
    );
}

#[tokio::test]
async fn rejects_non_audio_part() {
    let client = assemblyai("test-key");
    let err = client
        .transcription()
        .submit(vec![Part::text("transcribe this please")])
        .await
        .expect_err("a text part must be rejected pre-flight");
    assert!(
        err.to_string().contains("only audio parts"),
        "expected only-audio-parts validation error: {err}"
    );
}

#[tokio::test]
async fn requires_exactly_one_audio_part() {
    let client = assemblyai("test-key");
    let err = client
        .transcription()
        .submit(vec![
            Part::audio(ASSEMBLYAI_AUDIO_URL),
            Part::audio("https://storage.example.com/other.mp3"),
        ])
        .await
        .expect_err("two audio parts must be rejected pre-flight");
    assert!(
        err.to_string().contains("exactly one audio part"),
        "expected exactly-one-audio-part validation error: {err}"
    );

    let err = client
        .transcription()
        .submit(vec![])
        .await
        .expect_err("zero parts must be rejected pre-flight");
    assert!(err.to_string().contains("exactly one audio part"), "{err}");
}

#[tokio::test]
async fn unsupported_provider_rejected() {
    // Anthropic does not support transcription (OpenAI now does, ADR-051).
    let client = anthropic("test-key");
    let err = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect_err("an unsupported provider must be rejected");
    assert!(
        err.to_string().contains("does not support transcription"),
        "expected unsupported-provider error: {err}"
    );
}

// === Synchronous transcription — OpenAI (TranscriptionOpenAI, ADR-051) ===

const FAKE_MP3: &[u8] = &[0xFF, 0xFB, 0x90, 0x00, b'm', b'p', b'3'];

fn openai_verbose_transcript() -> serde_json::Value {
    serde_json::json!({
        "text": "The quarterly review is scheduled for Tuesday.",
        "segments": [
            { "start": 0.0, "end": 1.5, "text": "The quarterly review" },
            { "start": 1.5, "end": 2.84, "text": " is scheduled for Tuesday." },
        ],
    })
}

// Serves POST /v1/audio/transcriptions, asserting the multipart request shape
// (Bearer auth, multipart content-type, the model/response_format/file parts).
fn openai_transcription_server(response: serde_json::Value) -> String {
    common::serve_once(
        move |request: String, _body| {
            assert!(
                request.contains("POST /v1/audio/transcriptions"),
                "must POST the transcriptions endpoint: {request}"
            );
            let lower = request.to_ascii_lowercase();
            assert!(
                lower.contains("authorization: bearer test-key"),
                "must carry Bearer auth: {request}"
            );
            assert!(
                lower.contains("content-type: multipart/form-data; boundary="),
                "must send multipart/form-data: {request}"
            );
            assert!(request.contains("name=\"model\""), "missing model field");
            assert!(request.contains("whisper-1"), "missing model value");
            assert!(
                request.contains("name=\"response_format\""),
                "missing response_format field"
            );
            assert!(request.contains("verbose_json"), "missing response_format value");
            assert!(
                request.contains("name=\"file\"; filename=\"audio.mp3\""),
                "missing file part / filename: {request}"
            );
            assert!(
                request.to_ascii_lowercase().contains("content-type: audio/mpeg"),
                "missing file content-type: {request}"
            );
        },
        common::TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: response.to_string(),
            headers: vec![],
        },
    )
}

#[tokio::test]
async fn transcribe_sync_openai_segments_sec_to_ms() {
    let url = openai_transcription_server(openai_verbose_transcript());
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);

    let resp = client
        .transcription()
        .model("whisper-1")
        .transcribe(vec![Part::audio_bytes("audio/mpeg", FAKE_MP3.to_vec())])
        .await
        .expect("transcribe succeeds");

    assert_eq!(resp.text, "The quarterly review is scheduled for Tuesday.");
    assert_eq!(resp.segments.len(), 2);
    // verbose_json offsets are seconds; the segment stores integer ms.
    assert_eq!(resp.segments[0].end, 1500);
    assert_eq!(resp.segments[1].end, 2840);
}

#[tokio::test]
async fn transcribe_openai_empty_segments() {
    let url = openai_transcription_server(serde_json::json!({ "text": "Hello there." }));
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);

    let resp = client
        .transcription()
        .model("whisper-1")
        .transcribe(vec![Part::audio_bytes("audio/mpeg", FAKE_MP3.to_vec())])
        .await
        .expect("transcribe succeeds");
    assert_eq!(resp.text, "Hello there.");
    assert_eq!(resp.segments.len(), 0);
}

#[tokio::test]
async fn submit_on_sync_provider_rejected() {
    let err = openai("test-key")
        .transcription()
        .model("whisper-1")
        .submit(vec![Part::audio_bytes("audio/mpeg", FAKE_MP3.to_vec())])
        .await
        .expect_err("submit on a sync provider must be rejected");
    assert!(err.to_string().contains("Transcribe"), "must name Transcribe: {err}");
}

#[tokio::test]
async fn transcribe_on_async_provider_rejected() {
    let err = assemblyai("test-key")
        .transcription()
        .model("best")
        .transcribe(vec![Part::audio_bytes("audio/mpeg", FAKE_MP3.to_vec())])
        .await
        .expect_err("transcribe on an async provider must be rejected");
    assert!(
        err.to_string().contains("Submit/Wait"),
        "must name Submit/Wait: {err}"
    );
}

#[tokio::test]
async fn transcribe_rejects_audio_url() {
    let err = openai("test-key")
        .transcription()
        .model("whisper-1")
        .transcribe(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect_err("a remote audio URL must be rejected for OpenAI");
    assert!(err.to_string().contains("inline audio bytes"), "{err}");
}

#[tokio::test]
async fn transcribe_requires_model() {
    let err = openai("test-key")
        .transcription()
        .transcribe(vec![Part::audio_bytes("audio/mpeg", FAKE_MP3.to_vec())])
        .await
        .expect_err("a missing model must be rejected");
    assert!(err.to_string().contains("required for synchronous"), "{err}");
}
