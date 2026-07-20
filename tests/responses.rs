//
//
//
//
//
//

mod common;

use std::sync::{Arc, Mutex};

use common::{serve_once, TestResponse};
use llmkit::builders::{anthropic, openai};
use llmkit::Error;

//
//
fn request_path(request: &str) -> String {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_string()
}

//
//
//
//
#
async fn responses_parses_output_envelope() {
    let got_path = Arc::new(Mutex::new(String::new()));
    let got_path_in = got_path.clone();
    let base_url = serve_once(
        move |request, _json| {
            *got_path_in.lock().unwrap() = request_path(&request);
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "status": "completed",
                "output": [{
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Helsinki."}]
                }],
                "usage": {"input_tokens": 16, "output_tokens": 5}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .protocol("responses")
        .model("gpt-4o-mini")
        .prompt("capital of Finland?")
        .await
        .expect("responses prompt succeeds");

    assert_eq!(response.text, "Helsinki.");
    assert_eq!(response.usage.input, 16);
    assert_eq!(response.usage.output, 5);
    assert_eq!(response.finish_reason, "completed");
    assert_eq!(*got_path.lock().unwrap(), "/v1/responses");
}

//
//
//
#
async fn default_unchanged_hits_chat_completions() {
    let got_path = Arc::new(Mutex::new(String::new()));
    let got_path_in = got_path.clone();
    let base_url = serve_once(
        move |request, _json| {
            *got_path_in.lock().unwrap() = request_path(&request);
        },
        TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: serde_json::json!({
                "choices": [{"message": {"content": "Helsinki."}}],
                "usage": {"prompt_tokens": 16, "completion_tokens": 5}
            })
            .to_string(),
            headers: vec![],
        },
    );

    let mut client = openai("key");
    client.provider.base_url = Some(base_url);
    let response = client
        .text()
        .model("gpt-4o-mini")
        .prompt("capital of Finland?")
        .await
        .expect("default prompt succeeds");

    assert_eq!(response.text, "Helsinki.");
    assert_eq!(*got_path.lock().unwrap(), "/v1/chat/completions");
}

//
//
//
#
async fn unsupported_provider_errors() {
    let client = anthropic("key");
    let err = client
        .text()
        .protocol("responses")
        .model("claude-sonnet-4-6")
        .prompt("hi")
        .await
        .expect_err("unsupported protocol must error");

    match err {
        Error::Validation { field, .. } => assert_eq!(field, "protocol"),
        other => panic!("expected ValidationError(field:protocol), got {other:?}"),
    }
}

//
//
#
async fn unknown_protocol_errors() {
    let client = openai("key");
    let err = client
        .text()
        .protocol("nonexistent")
        .model("gpt-4o-mini")
        .prompt("hi")
        .await
        .expect_err("unknown protocol must error");

    match err {
        Error::Validation { field, .. } => assert_eq!(field, "protocol"),
        other => panic!("expected ValidationError(field:protocol), got {other:?}"),
    }
}
