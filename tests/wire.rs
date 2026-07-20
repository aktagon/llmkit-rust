//
//

use llmkit::{load_history, save_history, Message, ToolCall, ToolResult, WireError};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).parent().unwrap().to_path_buf()
}

fn golden_path() -> PathBuf {
    repo_root().join("codegen/testdata/wire/v1/messages.json")
}

fn canonical_fixture() -> Vec<Message> {
    vec![
        Message {
            role: "user".into(),
            content: "list .py files in src".into(),
            ..Default::default()
        },
        Message {
            role: "assistant".into(),
            content: "".into(),
            tool_calls: vec![ToolCall {
                id: "call_abc".into(),
                name: "list_files".into(),
                input: Some(serde_json::json!({"path": "src"})),
            }],
            tool_result: None,
        },
        Message {
            role: "tool".into(),
            content: "".into(),
            tool_calls: vec![],
            tool_result: Some(ToolResult {
                tool_use_id: "call_abc".into(),
                content: "a.py b.py".into(),
            }),
        },
        Message {
            role: "assistant".into(),
            content: "Found 2 Python files: a.py, b.py".into(),
            ..Default::default()
        },
    ]
}

#
fn wire_golden_matches() {
    //
    let fixture = canonical_fixture();
    let actual: Value = serde_json::from_str(&save_history(&fixture).unwrap()).unwrap();
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(golden_path()).unwrap()).unwrap();
    assert_eq!(actual, expected);
}

#
fn wire_round_trip_value_equal() {
    //
    let fixture = canonical_fixture();
    let bytes = save_history(&fixture).unwrap();
    let restored = load_history(&bytes).unwrap();
    assert_eq!(restored, fixture);
}

#
fn wire_missing_version_rejected() {
    //
    let err = load_history(r#"{"messages": []}"#).unwrap_err();
    assert!(matches!(err, WireError::MissingVersion));
}

#
fn wire_unsupported_version_rejected() {
    //
    let err = load_history(r#"{"_v": 99, "messages": []}"#).unwrap_err();
    assert!(matches!(err, WireError::UnsupportedVersion { .. }));
}

#
fn wire_unknown_top_level_key_rejected() {
    //
    let err = load_history(r#"{"_v": 1, "messages": [], "stray": 42}"#).unwrap_err();
    assert!(matches!(err, WireError::UnknownTopLevelKey(_)));
}

#
fn wire_meta_passthrough_accepted() {
    //
    let msgs = load_history(r#"{"_v": 1, "messages": [], "_meta": {"trace": "abc"}}"#).unwrap();
    assert!(msgs.is_empty());
}

#
fn wire_chain_method_load_accepts_canonical_bytes() {
    //
    //
    //
    //
    //
    //
    //
    use llmkit::builders::anthropic;
    let bytes = save_history(&canonical_fixture()).unwrap();
    let fresh = anthropic("k").agent().load(&bytes).unwrap();
    assert!(fresh.messages().is_empty());
}

#
fn wire_malformed_documents_rejected() {
    //
    //
    for input in [
        "[]",
        r#"{"_v": "1", "messages": []}"#,
        r#"{"_v": 1.5, "messages": []}"#,
        r#"{"_v": 1, "messages": "oops"}"#,
    ] {
        match load_history(input) {
            Err(WireError::Malformed(_)) => {}
            other => panic!("input {input:?} produced {other:?}, want Malformed"),
        }
    }
}

#
fn wire_drop_target_artifact() {
    //
    let dir = repo_root().join("target/wire");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("rust.json"),
        save_history(&canonical_fixture()).unwrap(),
    )
    .unwrap();
}
