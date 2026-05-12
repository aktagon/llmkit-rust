//! Unit tests for `PromptOptions` builder methods. The typed-builder
//! `Text` constructs its own internal `PromptOptions` by directly
//! setting fields (see `builders/text.rs::build_options`), bypassing
//! these chain methods — so they're never reached by integration
//! tests. They're real public API, though, available to callers who
//! construct `PromptOptions` directly. These tests exercise each
//! method + the `new()` and `Debug` impls.

use llmkit::PromptOptions;

#[test]
fn new_returns_default_options() {
    let opts = PromptOptions::new();
    assert!(!opts.caching);
    assert_eq!(opts.cache_ttl, None);
    assert_eq!(opts.temperature, None);
    assert_eq!(opts.top_p, None);
    assert_eq!(opts.top_k, None);
    assert_eq!(opts.max_tokens, None);
    assert!(opts.stop_sequences.is_empty());
    assert_eq!(opts.seed, None);
    assert_eq!(opts.frequency_penalty, None);
    assert_eq!(opts.presence_penalty, None);
    assert_eq!(opts.thinking_budget, None);
    assert_eq!(opts.reasoning_effort, None);
    assert!(opts.middleware.is_empty());
}

#[test]
fn caching_flips_flag_on() {
    let opts = PromptOptions::new().caching();
    assert!(opts.caching);
}

#[test]
fn cache_ttl_sets_seconds() {
    let opts = PromptOptions::new().cache_ttl(3600);
    assert_eq!(opts.cache_ttl, Some(3600));
}

#[test]
fn temperature_sets_value() {
    let opts = PromptOptions::new().temperature(0.7);
    assert_eq!(opts.temperature, Some(0.7));
}

#[test]
fn top_p_sets_value() {
    let opts = PromptOptions::new().top_p(0.9);
    assert_eq!(opts.top_p, Some(0.9));
}

#[test]
fn top_k_sets_value() {
    let opts = PromptOptions::new().top_k(40);
    assert_eq!(opts.top_k, Some(40));
}

#[test]
fn max_tokens_sets_value() {
    let opts = PromptOptions::new().max_tokens(2048);
    assert_eq!(opts.max_tokens, Some(2048));
}

#[test]
fn stop_sequences_collects_into_owned_strings() {
    // Caller can pass anything that yields Into<String>. Verify both &str
    // and String inputs work (the trait bound is the API contract).
    let opts = PromptOptions::new().stop_sequences(["END", "STOP"]);
    assert_eq!(
        opts.stop_sequences,
        vec!["END".to_string(), "STOP".to_string()]
    );

    let opts2 = PromptOptions::new().stop_sequences(vec![String::from("a"), String::from("b")]);
    assert_eq!(opts2.stop_sequences, vec!["a", "b"]);
}

#[test]
fn seed_sets_value() {
    let opts = PromptOptions::new().seed(42);
    assert_eq!(opts.seed, Some(42));
}

#[test]
fn frequency_penalty_sets_value() {
    let opts = PromptOptions::new().frequency_penalty(0.1);
    assert_eq!(opts.frequency_penalty, Some(0.1));
}

#[test]
fn presence_penalty_sets_value() {
    let opts = PromptOptions::new().presence_penalty(0.2);
    assert_eq!(opts.presence_penalty, Some(0.2));
}

#[test]
fn thinking_budget_sets_value() {
    let opts = PromptOptions::new().thinking_budget(1024);
    assert_eq!(opts.thinking_budget, Some(1024));
}

#[test]
fn reasoning_effort_accepts_into_string() {
    let opts = PromptOptions::new().reasoning_effort("medium");
    assert_eq!(opts.reasoning_effort, Some("medium".to_string()));

    let opts2 = PromptOptions::new().reasoning_effort(String::from("high"));
    assert_eq!(opts2.reasoning_effort, Some("high".to_string()));
}

#[test]
fn chain_methods_compose_left_to_right() {
    // Each chain method takes `mut self` and returns `Self`. Verify
    // that chaining multiple methods accumulates state without
    // clobbering earlier setters.
    let opts = PromptOptions::new()
        .temperature(0.5)
        .max_tokens(100)
        .caching()
        .cache_ttl(600)
        .stop_sequences(["END"]);

    assert_eq!(opts.temperature, Some(0.5));
    assert_eq!(opts.max_tokens, Some(100));
    assert!(opts.caching);
    assert_eq!(opts.cache_ttl, Some(600));
    assert_eq!(opts.stop_sequences, vec!["END"]);
}

#[test]
fn debug_impl_renders_middleware_as_count_summary() {
    // The hand-written Debug impl renders the middleware vec as
    // "[N fns]" — verify a fresh PromptOptions shows "[0 fns]".
    let opts = PromptOptions::new();
    let s = format!("{:?}", opts);
    assert!(s.contains("[0 fns]"), "expected '[0 fns]' marker in: {}", s);
    // And the major fields are part of the printout.
    assert!(s.contains("PromptOptions"));
    assert!(s.contains("temperature"));
}

#[test]
fn clone_produces_independent_copy() {
    let original = PromptOptions::new().max_tokens(50).temperature(0.3);
    let cloned = original.clone();
    assert_eq!(cloned.max_tokens, Some(50));
    assert_eq!(cloned.temperature, Some(0.3));
}
