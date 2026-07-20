//
//
//
//
//
//
//
//
//
//
//
//
//

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

//
//
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

//
//
//
//
//
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

    //
    //
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

#
async fn lifecycle_batch_succeeded_golden() {
    //
    //
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

#
async fn lifecycle_batch_failed_golden() {
    //
    //
    let url = serve_sequence(vec![any_exchange(
        serde_json::json!({ "id": "batch_1", "status": "failed" }).to_string(),
    )]);

    let st = openai_batch_handle(url).poll().await.expect("poll succeeds");
    assert_lifecycle_golden("batch-failed", &st);
}
