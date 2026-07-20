//
//
//
//
//
//
//

mod common;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use common::{serve_sequence, TestExchange, TestResponse};
use llmkit::builders::{assemblyai, openai, BatchHandleExt, TranscriptionHandleExt};
use llmkit::{wait_batch, BatchHandle, BatchPoll, Error, JobState, Part, Provider, PromptOptions};

const ASSEMBLYAI_AUDIO_URL: &str = "https://storage.example.com/meeting.mp3";

fn json_response(body: serde_json::Value) -> TestResponse {
    TestResponse {
        status_line: "HTTP/1.1 200 OK",
        body: body.to_string(),
        headers: Vec::new(),
    }
}

fn any_exchange(body: serde_json::Value) -> TestExchange {
    TestExchange {
        assert_request: Box::new(|_request, _body| {}),
        response: json_response(body),
    }
}

fn completed_transcript() -> serde_json::Value {
    serde_json::json!({
        "id": "transcript-7c2",
        "status": "completed",
        "text": "The quarterly review is scheduled for Tuesday.",
        "words": [
            { "text": "The", "start": 120, "end": 280, "speaker": "A" },
            { "text": "quarterly", "start": 280, "end": 760 },
        ],
    })
}

//
//
//
fn serve_status_forever(body: serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let payload = body.to_string();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => continue,
            };
            let mut buf = [0_u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}")
}

fn openai_batch_handle(url: String) -> BatchHandle {
    let mut client = openai("test-key");
    client.provider.base_url = Some(url);
    BatchHandle {
        id: "batch_1".into(),
        provider: Provider {
            name: client.provider.name,
            api_key: client.provider.api_key.clone(),
            model: None,
            base_url: client.provider.base_url.clone(),
            headers: client.provider.headers.clone(),
        },
        raw: false,
    }
}

fn fast_batch_poll(timeout: Duration) -> BatchPoll {
    BatchPoll {
        interval: Duration::from_millis(1),
        timeout,
    }
}

//

#
fn job_state_display() {
    assert_eq!(JobState::Running.to_string(), "running");
    assert_eq!(JobState::Succeeded.to_string(), "succeeded");
    assert_eq!(JobState::Failed.to_string(), "failed");
}

//

//
//
#
async fn transcription_poll_succeeded() {
    let url = serve_sequence(vec![
        any_exchange(serde_json::json!({ "id": "transcript-7c2", "status": "queued" })),
        any_exchange(completed_transcript()),
    ]);
    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);
    let handle = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect("submit succeeds");

    let st = handle.poll().await.expect("poll succeeds");
    assert_eq!(st.state, JobState::Succeeded);
    assert_eq!(st.raw_status, "completed");
    assert!(st.cause.is_none(), "no cause on success");
    let result = st.result.expect("result populated on success");
    assert_eq!(result.text, "The quarterly review is scheduled for Tuesday.");
}

//
//
#
async fn transcription_poll_running() {
    let url = serve_sequence(vec![
        any_exchange(serde_json::json!({ "id": "transcript-7c2", "status": "queued" })),
        any_exchange(serde_json::json!({ "id": "transcript-7c2", "status": "processing" })),
    ]);
    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);
    let handle = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect("submit succeeds");

    let st = handle.poll().await.expect("poll succeeds");
    assert_eq!(st.state, JobState::Running);
    assert_eq!(st.raw_status, "processing");
    assert!(st.result.is_none() && st.cause.is_none());
}

//
//
#
async fn transcription_poll_failed() {
    let failed = serde_json::json!({
        "id": "transcript-7c2",
        "status": "error",
        "error": "Download error, unable to download https://storage.example.com/meeting.mp3",
    });
    let url = serve_sequence(vec![
        any_exchange(serde_json::json!({ "id": "transcript-7c2", "status": "queued" })),
        any_exchange(failed),
    ]);
    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);
    let handle = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect("submit succeeds");

    let st = handle.poll().await.expect("poll succeeds");
    assert_eq!(st.state, JobState::Failed);
    assert!(st.result.is_none(), "no result on failure");
    let cause = st.cause.expect("cause populated on failure");
    assert_eq!(cause.status, "error");
    assert!(
        cause.message.contains("Download error"),
        "cause carries the provider message: {}",
        cause.message
    );
    assert!(!cause.timed_out, "provider failure is not a timeout");
}

//
//
#
async fn transcription_wait_failed_error_message() {
    let failed = serde_json::json!({
        "id": "transcript-7c2",
        "status": "error",
        "error": "Download error, unable to download the source audio",
    });
    let url = serve_sequence(vec![
        any_exchange(serde_json::json!({ "id": "transcript-7c2", "status": "queued" })),
        any_exchange(failed),
    ]);
    let mut client = assemblyai("test-key");
    client.provider.base_url = Some(url);
    let handle = client
        .transcription()
        .submit(vec![Part::audio(ASSEMBLYAI_AUDIO_URL)])
        .await
        .expect("submit succeeds");

    let err = handle.wait().await.expect_err("a failed transcription errors");
    let msg = err.to_string();
    assert!(
        msg.contains("transcription failed: ") && msg.contains("Download error"),
        "wait error format: got {msg:?}"
    );
    assert!(
        !matches!(err, Error::PollTimeout { .. }),
        "a provider failure must not be a PollTimeout"
    );
}

//

//
//
#
async fn batch_poll_running() {
    let url = serve_sequence(vec![any_exchange(
        serde_json::json!({ "id": "batch_1", "status": "in_progress" }),
    )]);
    let handle = openai_batch_handle(url);

    let st = handle.poll().await.expect("poll succeeds");
    assert_eq!(st.state, JobState::Running);
    assert_eq!(st.raw_status, "in_progress");
    assert!(st.result.is_none());
}

//
//
//
#
async fn batch_poll_failed() {
    let url = serve_sequence(vec![any_exchange(
        serde_json::json!({ "id": "batch_1", "status": "failed" }),
    )]);
    let handle = openai_batch_handle(url);

    let st = handle.poll().await.expect("poll succeeds");
    assert_eq!(st.state, JobState::Failed);
    assert!(st.result.is_none());
    let cause = st.cause.expect("cause populated on failure");
    assert_eq!(cause.status, "failed");
    assert!(!cause.timed_out);
}

//
//
#
async fn batch_wait_failed_error() {
    let url = serve_status_forever(serde_json::json!({ "id": "batch_1", "status": "expired" }));
    let handle = openai_batch_handle(url);

    let err = wait_batch(&handle, PromptOptions::new(), fast_batch_poll(Duration::from_secs(3)))
        .await
        .expect_err("a failed batch errors");
    assert!(
        err.to_string().contains("batch failed: "),
        "wait error format: got {:?}",
        err.to_string()
    );
    assert!(
        !matches!(err, Error::PollTimeout { .. }),
        "a provider failure must not be a PollTimeout"
    );
}

//
//
//
#
async fn batch_wait_times_out_at_backstop() {
    let url = serve_status_forever(serde_json::json!({ "id": "batch_1", "status": "in_progress" }));
    let handle = openai_batch_handle(url);

    let err = wait_batch(
        &handle,
        PromptOptions::new(),
        fast_batch_poll(Duration::from_millis(20)),
    )
    .await
    .expect_err("expected the deadline backstop to fire");
    match err {
        Error::PollTimeout { id, .. } => assert_eq!(id, "batch_1"),
        other => panic!("expected Error::PollTimeout, got {other:?}"),
    }
}
