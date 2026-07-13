// Cross-SDK LIFECYCLE conformance (ADR-062 slice 1 / HANDOFF-032 step 5).
// Sibling of request_wire.rs. Where that suite asserts the OUTBOUND request
// bytes are identical across the four SDKs, this asserts the INBOUND
// classification is: given the same provider poll response, every SDK's job
// engine normalizes it to the SAME terminal JobStatus. Go's lifecycle_wire_test.go
// is the FROZEN reference; this driver drops target/wire/lifecycle/<fixture>/rust.json
// value-equal to the shared golden at
// codegen/testdata/wire/lifecycle/v1/<fixture>.json.
//
// Each fixture spins the stdlib-TCP mock (common::serve_sequence) serving the
// OpenAI two-hop batch shape, calls BatchHandle::poll() ONCE (one round-trip, no
// wait loop), and normalizes the returned JobStatus to {state, hasResult,
// rawStatus, cause}. codegen/test_cross_sdk_lifecycle.py compares.

mod common;

use common::{serve_sequence, TestExchange, TestResponse};
use llmkit::builders::{openai, BatchHandleExt};
use llmkit::{BatchHandle, JobStatus, Provider, Response};

fn json_response(body: String) -> TestResponse {
    TestResponse {
        status_line: "HTTP/1.1 200 OK",
        body,
        headers: Vec::new(),
    }
}

fn any_exchange(body: String) -> TestExchange {
    TestExchange {
        assert_request: Box::new(|_request, _body| {}),
        response: json_response(body),
    }
}

// An OpenAI batch handle pointed at the mock base URL (id "batch_1", key
// "test-key") — mirror of job.rs::openai_batch_handle.
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

// Normalizes a terminal JobStatus to the cross-SDK-comparable projection and
// writes it to target/wire/lifecycle/<fixture>/rust.json. JobState -> the
// lowercase wire string via Display (matches Go's JobState.String()); the
// timed_out field is emitted under the camelCase key "timedOut" to match the
// golden; cause is null unless Failed.
fn assert_lifecycle_golden(fixture: &str, st: &JobStatus<Vec<Response>>) {
    let cause = match st.cause.as_ref() {
        Some(c) => serde_json::json!({ "status": c.status, "timedOut": c.timed_out }),
        None => serde_json::Value::Null,
    };
    let artifact = serde_json::json!({
        "state": st.state.to_string(),
        "hasResult": st.result.is_some(),
        "rawStatus": st.raw_status,
        "cause": cause,
    });

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root");
    let path = repo_root.join(format!("target/wire/lifecycle/{fixture}/rust.json"));
    std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir artifact dir");
    std::fs::write(&path, serde_json::to_string_pretty(&artifact).unwrap())
        .expect("write artifact");

    // Assert value-equality against the shared golden in-driver too, so a drift
    // fails cargo test directly (make check excludes Rust).
    let golden_path =
        repo_root.join(format!("codegen/testdata/wire/lifecycle/v1/{fixture}.json"));
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("read golden"))
            .expect("parse golden");
    assert_eq!(
        artifact, golden,
        "Rust lifecycle {fixture} differs from shared golden"
    );
}

#[tokio::test]
async fn lifecycle_batch_succeeded_golden() {
    // Two-hop: status GET reports completed + output_file_id, then the file
    // content GET returns one JSONL result line (OpenAI response.body shape).
    let jsonl = serde_json::json!({
        "custom_id": "req-0",
        "response": { "body": {
            "choices": [{ "message": { "role": "assistant", "content": "ok" } }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1 }
        }}
    })
    .to_string();
    let url = serve_sequence(vec![
        any_exchange(
            serde_json::json!({
                "id": "batch_1",
                "status": "completed",
                "output_file_id": "file-out-1"
            })
            .to_string(),
        ),
        any_exchange(jsonl),
    ]);

    let st = openai_batch_handle(url).poll().await.expect("poll succeeds");
    assert_lifecycle_golden("batch-succeeded", &st);
}

#[tokio::test]
async fn lifecycle_batch_failed_golden() {
    // status GET reports failed and there is no output_file_id — one round-trip,
    // no result fetch.
    let url = serve_sequence(vec![any_exchange(
        serde_json::json!({ "id": "batch_1", "status": "failed" }).to_string(),
    )]);

    let st = openai_batch_handle(url).poll().await.expect("poll succeeds");
    assert_lifecycle_golden("batch-failed", &st);
}
