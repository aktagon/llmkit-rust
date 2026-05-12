//! Unit tests for the publicly reachable surface of `MiddlewareVeto`:
//! Display, Error::source, Debug. The runtime helpers `fire_pre` /
//! `fire_post` are crate-private (the `middleware` module is private;
//! only the types are re-exported); they're exercised indirectly by
//! the typed-builder integration tests under `prompt_middleware_*`
//! and `upload_middleware_*` in tests/prompt.rs.

use std::error::Error as StdError;

use llmkit::MiddlewareVeto;

#[test]
fn veto_display_prefixes_cause_with_marker() {
    let cause: Box<dyn StdError + Send + Sync> = "budget exceeded".into();
    let veto = MiddlewareVeto { cause };
    assert_eq!(format!("{}", veto), "middleware veto: budget exceeded");
}

#[test]
fn veto_error_source_returns_underlying_cause() {
    // io::Error is the kind of structured cause middleware authors
    // wrap when they want callers to downcast on it.
    let cause: Box<dyn StdError + Send + Sync> =
        std::io::Error::new(std::io::ErrorKind::Other, "network down").into();
    let veto = MiddlewareVeto { cause };
    let src = veto.source().expect("source should be the wrapped cause");
    assert_eq!(src.to_string(), "network down");
}

#[test]
fn veto_debug_renders_struct_name() {
    let cause: Box<dyn StdError + Send + Sync> = "x".into();
    let veto = MiddlewareVeto { cause };
    let s = format!("{:?}", veto);
    assert!(
        s.contains("MiddlewareVeto"),
        "debug missing struct name: {}",
        s
    );
}
