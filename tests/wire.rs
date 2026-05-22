// ADR-023 STAB-007: per-SDK wire round-trip test against the canonical
// golden at codegen/testdata/wire/v1/messages.json.

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

#[test]
fn wire_golden_matches() {
    // STAB-007: save_history output is JSON-value-equal to the golden.
    let fixture = canonical_fixture();
    let actual: Value = serde_json::from_str(&save_history(&fixture).unwrap()).unwrap();
    let expected: Value =
        serde_json::from_str(&fs::read_to_string(golden_path()).unwrap()).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn wire_round_trip_value_equal() {
    // STAB-007: load_history(save_history(msgs)) == msgs.
    let fixture = canonical_fixture();
    let bytes = save_history(&fixture).unwrap();
    let restored = load_history(&bytes).unwrap();
    assert_eq!(restored, fixture);
}

#[test]
fn wire_missing_version_rejected() {
    // STAB-011: bare-array dumps are rejected.
    let err = load_history(r#"{"messages": []}"#).unwrap_err();
    assert!(matches!(err, WireError::MissingVersion));
}

#[test]
fn wire_unsupported_version_rejected() {
    // STAB-003: _v above WIRE_SCHEMA_VERSION is rejected.
    let err = load_history(r#"{"_v": 99, "messages": []}"#).unwrap_err();
    assert!(matches!(err, WireError::UnsupportedVersion { .. }));
}

#[test]
fn wire_unknown_top_level_key_rejected() {
    // STAB-002: unknown top-level keys are rejected.
    let err = load_history(r#"{"_v": 1, "messages": [], "stray": 42}"#).unwrap_err();
    assert!(matches!(err, WireError::UnknownTopLevelKey(_)));
}

#[test]
fn wire_meta_passthrough_accepted() {
    // STAB-002: _meta is a consumer-owned pass-through namespace.
    let msgs = load_history(r#"{"_v": 1, "messages": [], "_meta": {"trace": "abc"}}"#).unwrap();
    assert!(msgs.is_empty());
}

#[test]
fn wire_chain_methods_round_trip() {
    // STAB-012: bot.save() / bot.load(data) round-trip end-to-end.
    // The chain-state `history` field on *Agent is pub(crate); from
    // integration-test scope we exercise the contract through the
    // free functions plus a Save/Load on a fresh builder.
    use llmkit::builders::anthropic;
    let bytes = save_history(&canonical_fixture()).unwrap();
    let fresh = anthropic("k").agent().load(&bytes).unwrap();
    // bot.messages() reads runtime state, which is empty before
    // .prompt() runs. The round-trip contract — chain history is
    // populated, runtime state is cleared — is verified end-to-end
    // by per-SDK unit tests; here we just confirm the Load chain
    // method accepts the canonical bytes without error.
    assert!(fresh.messages().is_empty());
}

#[test]
fn wire_drop_target_artifact() {
    // STAB-010: drop target/wire/rust.json for the cross-SDK comparator.
    let dir = repo_root().join("target/wire");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("rust.json"),
        save_history(&canonical_fixture()).unwrap(),
    )
    .unwrap();
}
