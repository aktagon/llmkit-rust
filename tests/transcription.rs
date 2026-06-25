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
use llmkit::builders::assemblyai;
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
    let client = llmkit::builders::openai("test-key");
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
