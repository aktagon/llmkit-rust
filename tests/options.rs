//!
//!
//!
//!
//!
//!
//!

use llmkit::PromptOptions;

#
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

#
fn caching_flips_flag_on() {
    let opts = PromptOptions::new().caching();
    assert!(opts.caching);
}

#
fn cache_ttl_sets_seconds() {
    let opts = PromptOptions::new().cache_ttl(3600);
    assert_eq!(opts.cache_ttl, Some(3600));
}

#
fn temperature_sets_value() {
    let opts = PromptOptions::new().temperature(0.7);
    assert_eq!(opts.temperature, Some(0.7));
}

#
fn top_p_sets_value() {
    let opts = PromptOptions::new().top_p(0.9);
    assert_eq!(opts.top_p, Some(0.9));
}

#
fn top_k_sets_value() {
    let opts = PromptOptions::new().top_k(40);
    assert_eq!(opts.top_k, Some(40));
}

#
fn max_tokens_sets_value() {
    let opts = PromptOptions::new().max_tokens(2048);
    assert_eq!(opts.max_tokens, Some(2048));
}

#
fn stop_sequences_collects_into_owned_strings() {
    //
    //
    let opts = PromptOptions::new().stop_sequences(["END", "STOP"]);
    assert_eq!(
        opts.stop_sequences,
        vec!["END".to_string(), "STOP".to_string()]
    );

    let opts2 = PromptOptions::new().stop_sequences(vec![String::from("a"), String::from("b")]);
    assert_eq!(opts2.stop_sequences, vec!["a", "b"]);
}

#
fn seed_sets_value() {
    let opts = PromptOptions::new().seed(42);
    assert_eq!(opts.seed, Some(42));
}

#
fn frequency_penalty_sets_value() {
    let opts = PromptOptions::new().frequency_penalty(0.1);
    assert_eq!(opts.frequency_penalty, Some(0.1));
}

#
fn presence_penalty_sets_value() {
    let opts = PromptOptions::new().presence_penalty(0.2);
    assert_eq!(opts.presence_penalty, Some(0.2));
}

#
fn thinking_budget_sets_value() {
    let opts = PromptOptions::new().thinking_budget(1024);
    assert_eq!(opts.thinking_budget, Some(1024));
}

#
fn reasoning_effort_accepts_into_string() {
    let opts = PromptOptions::new().reasoning_effort("medium");
    assert_eq!(opts.reasoning_effort, Some("medium".to_string()));

    let opts2 = PromptOptions::new().reasoning_effort(String::from("high"));
    assert_eq!(opts2.reasoning_effort, Some("high".to_string()));
}

#
fn chain_methods_compose_left_to_right() {
    //
    //
    //
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

#
fn debug_impl_renders_middleware_as_count_summary() {
    //
    //
    let opts = PromptOptions::new();
    let s = format!("{:?}", opts);
    assert!(s.contains("[0 fns]"), "expected '[0 fns]' marker in: {}", s);
    //
    assert!(s.contains("PromptOptions"));
    assert!(s.contains("temperature"));
}

#
fn clone_produces_independent_copy() {
    let original = PromptOptions::new().max_tokens(50).temperature(0.3);
    let cloned = original.clone();
    assert_eq!(cloned.max_tokens, Some(50));
    assert_eq!(cloned.temperature, Some(0.3));
}
