//! Crate-internal unit tests for the typed-builder chain-method
//! contract. These tests inspect `pub(crate)` builder fields directly
//! to verify that each chain method lands its argument in the expected
//! slot — they're testing implementation invariants, not the user-
//! facing API. Integration tests in `tests/builders.rs` cover the
//! latter.
//!
//! Moved from `tests/builders.rs` in plan 020 when builder fields were
//! locked down to `pub(crate)` to keep the public SemVer surface
//! minimal.

#![cfg(test)]

use std::sync::Arc;

use super::{agent::AgentState, anthropic, google, Client};
use crate::middleware::MiddlewareFn;
use crate::structs::Message;
use crate::types::Tool;
use crate::ProviderName;

fn noop_middleware() -> MiddlewareFn {
    Arc::new(|_event| None)
}

#[test]
fn text_chain_lands_in_fields() {
    let mw = noop_middleware();
    let text = google("k")
        .text()
        .caching()
        .file("file-id")
        .history(vec![Message::new("user", "earlier")])
        .image("image/png", vec![0xff])
        .max_tokens(42)
        .add_middleware(vec![mw])
        .model("text-model")
        .schema(r#"{"type":"object"}"#)
        .system("you are a tutor")
        .temperature(0.7)
        .text("hello");

    assert!(text.caching);
    assert_eq!(text.files.len(), 1);
    assert_eq!(text.files[0].id, "file-id");
    assert_eq!(text.history.len(), 1);
    assert_eq!(text.history[0].content, "earlier");
    assert_eq!(text.max_tokens, Some(42));
    assert_eq!(text.middleware.len(), 1);
    assert_eq!(text.model.as_deref(), Some("text-model"));
    assert_eq!(text.schema.as_deref(), Some(r#"{"type":"object"}"#));
    assert_eq!(text.system.as_deref(), Some("you are a tutor"));
    assert_eq!(text.temperature, Some(0.7));
    assert_eq!(text.parts.len(), 2);
    // Part ordering preserved: image (added first) precedes text.
    match &text.parts[0] {
        crate::Part::Image(crate::MediaRef { mime_type, bytes }) => {
            assert_eq!(mime_type, "image/png");
            assert_eq!(bytes, &vec![0xff_u8]);
        }
        _ => panic!("parts[0] not an image"),
    }
    match &text.parts[1] {
        crate::Part::Text(s) => assert_eq!(s, "hello"),
        _ => panic!("parts[1] not text"),
    }
}

#[test]
fn image_chain_lands_in_fields() {
    let img = google("k")
        .image()
        .aspect_ratio("16:9")
        .image("image/png", vec![0xff])
        .image_size("2K")
        .include_text()
        .add_middleware(vec![noop_middleware()])
        .model("img-model")
        .text("compose");

    assert_eq!(img.aspect_ratio.as_deref(), Some("16:9"));
    assert_eq!(img.image_size.as_deref(), Some("2K"));
    assert!(img.include_text);
    assert_eq!(img.middleware.len(), 1);
    assert_eq!(img.model.as_deref(), Some("img-model"));
    assert_eq!(img.parts.len(), 2);
}

#[test]
fn agent_chain_lands_in_fields() {
    let tool = Tool::new("calc", "calculator", serde_json::json!({}), |_args| {
        Ok("42".to_string())
    });
    let ag = google("k")
        .agent()
        .caching()
        .max_tokens(1)
        .max_tool_iterations(3)
        .add_middleware(vec![noop_middleware()])
        .model("a")
        .system("sys")
        .temperature(0.5)
        .add_tool(tool);

    assert!(ag.caching);
    assert_eq!(ag.max_tokens, Some(1));
    assert_eq!(ag.max_tool_iterations, Some(3));
    assert_eq!(ag.middleware.len(), 1);
    assert_eq!(ag.model.as_deref(), Some("a"));
    assert_eq!(ag.system.as_deref(), Some("sys"));
    assert_eq!(ag.temperature, Some(0.5));
    assert_eq!(ag.tools.len(), 1);
    assert_eq!(ag.tools[0].name, "calc");
}

#[test]
fn upload_chain_lands_in_fields() {
    let up = google("k")
        .upload()
        .bytes(b"hi".to_vec())
        .filename("f")
        .add_middleware(vec![noop_middleware()])
        .mime_type("text/plain")
        .path("/tmp/x");

    assert_eq!(up.bytes, b"hi".to_vec());
    assert_eq!(up.filename.as_deref(), Some("f"));
    assert_eq!(up.middleware.len(), 1);
    assert_eq!(up.mime_type.as_deref(), Some("text/plain"));
    assert_eq!(up.path.as_deref(), Some("/tmp/x"));
}

#[test]
fn client_text_method_returns_fresh_builder_each_call() {
    let c = google("k");
    let a = c.text().system("first");
    let b = c.text().system("second");
    assert_eq!(a.system.as_deref(), Some("first"));
    assert_eq!(b.system.as_deref(), Some("second"));
}

/// Load-bearing contract test for `RUST_BUILDER_POST_MUTATION["Agent"]`:
/// without the `out.state = None` hook, a forked clone via
/// `bot.system("new")` would silently share its parent's history
/// through the same AgentState reference.
#[test]
fn agent_reset_clears_state() {
    use crate::types::Provider;
    let mut bot = anthropic("k").agent().system("s");
    let provider = Provider::new(ProviderName::Anthropic, "k");
    bot.state = Some(AgentState::placeholder(provider));
    bot.reset();
    assert!(bot.state.is_none());
}

#[test]
fn agent_state_forking_load_bearing() {
    use crate::types::Provider;
    let bot = anthropic("k").agent().system("orig");
    let provider = Provider::new(ProviderName::Anthropic, "k");
    let mut bot = bot;
    bot.state = Some(AgentState::placeholder(provider));

    let forked = bot.system("new");
    // Rust's ownership consumed `bot` — the contract is on the FORK:
    // chain methods produce a fresh-state clone, so `forked.state` must
    // be None even though we set the parent's state to Some(...).
    assert!(forked.state.is_none());
}

#[test]
fn agent_history_writer_replaces_chain_state() {
    // ADR-020 HIST-003: bot.history(msgs) replaces (not appends) the
    // chain history list.
    use crate::structs::Message;
    let m_a = Message {
        role: "user".into(),
        content: "first".into(),
        ..Default::default()
    };
    let m_b = Message {
        role: "assistant".into(),
        content: "ok".into(),
        ..Default::default()
    };
    let bot = anthropic("k")
        .agent()
        .history(vec![m_a.clone(), m_b.clone()]);
    assert_eq!(bot.history.len(), 2);
    let m_c = Message {
        role: "user".into(),
        content: "reset".into(),
        ..Default::default()
    };
    let rebot = bot.history(vec![m_c.clone()]);
    assert_eq!(rebot.history, vec![m_c]);
}

#[test]
fn agent_messages_reader_empty_before_prompt() {
    // ADR-020 HIST-004: bot.messages() returns an empty Vec before
    // .prompt() initializes runtime state.
    use crate::structs::Message;
    let m = Message {
        role: "user".into(),
        content: "hi".into(),
        ..Default::default()
    };
    let bot = anthropic("k").agent().history(vec![m]);
    assert!(bot.messages().is_empty());
}

#[test]
fn agent_chain_methods_round_trip() {
    // ADR-023 STAB-012: bot.save() / bot.load(data) round-trip
    // end-to-end. This test reaches into pub(crate) `history` and
    // `state` fields, so it lives here rather than in
    // tests/wire.rs. The contract: load() populates `history`
    // from the wire bytes AND clears `state` so the next .prompt()
    // re-inits from the loaded history.
    use crate::structs::{Message, ToolCall, ToolResult};
    use crate::wire::save_history;

    let fixture = vec![
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
    ];

    let bytes = save_history(&fixture).unwrap();
    let bot = anthropic("k").agent();
    // Pre-condition: a fresh builder has no runtime state.
    assert!(bot.state.is_none());

    let loaded = bot.load(&bytes).unwrap();
    // STAB-012 contract: history populated, state cleared.
    assert_eq!(loaded.history, fixture);
    assert!(loaded.state.is_none());

    // And the loaded chain history projects through bot.messages()
    // once a runtime state is initialized — but here we just
    // assert the data path land in `.history`. Live-agent path is
    // covered by the integration tests in tests/wire.rs.
}

/// Appender semantics (ADR-021): two add_tool calls accumulate, not
/// replace. Regression guard against any future "simplification" of
/// the chain body to assignment.
#[test]
fn agent_add_tool_appends() {
    let t1 = Tool::new("first", "d", serde_json::json!({}), |_args| {
        Ok(String::new())
    });
    let t2 = Tool::new("second", "d", serde_json::json!({}), |_args| {
        Ok(String::new())
    });
    let ag = google("k").agent().system("S").add_tool(t1).add_tool(t2);
    assert_eq!(ag.tools.len(), 2);
    assert_eq!(ag.tools[0].name, "first");
    assert_eq!(ag.tools[1].name, "second");
}

/// Mirrors agent_add_tool_appends for add_middleware.
#[test]
fn text_add_middleware_appends() {
    let bot = google("k")
        .text()
        .add_middleware(vec![noop_middleware()])
        .add_middleware(vec![noop_middleware()]);
    assert_eq!(bot.middleware.len(), 2);
}

// Compile check: the public type aliases stay constructible from
// outside via the typed-builder factory methods (no field access).
#[test]
fn type_aliases_constructible() {
    let _: crate::builders::ImageData = crate::builders::ImageData::default();
    let _: crate::builders::MediaRef = crate::builders::MediaRef::default();
    let _: Client = google("k");
}
