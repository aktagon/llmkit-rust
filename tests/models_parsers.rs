//! Parser tests against codegen/fixtures/models/*. Mirror of Go
//! go/providers/models_parsers_test.go, TS ts/tests/models_parsers.test.ts,
//! and Python python/tests/test_models_parsers.py.

use std::path::PathBuf;

use llmkit::providers::generated::models_parsers::{
    parse_anthropic_models_response, parse_google_models_response,
    parse_openai_cohort_models_response,
};

fn fixture(name: &str) -> Vec<u8> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop(); // workspace root
    path.push("codegen");
    path.push("fixtures");
    path.push("models");
    path.push(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()))
}

#[test]
fn parse_anthropic_fixture_records_and_metadata() {
    let page = parse_anthropic_models_response(&fixture("anthropic.json")).unwrap();
    assert_eq!(page.records.len(), 9);
    let first = &page.records[0];
    assert!(!first.id.is_empty());
    assert!(!first.display_name.is_empty());
    assert!(first.context_window > 0);
    assert!(first.max_output > 0);
}

#[test]
fn parse_anthropic_round_trips_raw() {
    let page = parse_anthropic_models_response(&fixture("anthropic.json")).unwrap();
    assert!(page.records[0].raw.is_some());
}

#[test]
fn parse_openai_cohort_fixture_records_and_no_pagination() {
    let page = parse_openai_cohort_models_response(&fixture("openai.json")).unwrap();
    assert_eq!(page.records.len(), 124);
    assert_eq!(page.next_cursor, "");
    assert!(!page.records[0].id.is_empty());
    assert!(page.records[0].created > 0);
}

#[test]
fn parse_anthropic_malformed_created_at_yields_zero() {
    // Documents the silent-failure contract: a bad timestamp does not
    // crash the parser; the record just lands with `created: 0`.
    // Future "improve error reporting" PRs must break this test
    // intentionally, not accidentally.
    let body = br#"{"data":[{"id":"m","created_at":"not-a-date"}]}"#;
    let page = parse_anthropic_models_response(body).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].created, 0);
}

#[test]
fn parse_google_fixture_strips_models_prefix() {
    let page = parse_google_models_response(&fixture("google.json")).unwrap();
    assert_eq!(page.records.len(), 50);
    for r in &page.records {
        assert!(!r.id.is_empty());
        assert!(!r.id.starts_with("models/"));
    }
    assert!(page.records.iter().any(|r| r.context_window > 0));
}
