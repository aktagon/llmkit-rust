//!
//!
//!

use std::path::PathBuf;

use llmkit::models_parsers::{
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

#
fn parse_anthropic_fixture_records_and_metadata() {
    let page = parse_anthropic_models_response(&fixture("anthropic.json")).unwrap();
    assert_eq!(page.records.len(), 9);
    let first = &page.records[0];
    assert!(!first.id.is_empty());
    assert!(!first.display_name.is_empty());
    assert!(first.context_window > 0);
    assert!(first.max_output > 0);
}

#
fn parse_anthropic_round_trips_raw() {
    let page = parse_anthropic_models_response(&fixture("anthropic.json")).unwrap();
    assert!(page.records[0].raw.is_some());
}

#
fn parse_openai_cohort_fixture_records_and_no_pagination() {
    let page = parse_openai_cohort_models_response(&fixture("openai.json")).unwrap();
    assert_eq!(page.records.len(), 124);
    assert_eq!(page.next_cursor, "");
    assert!(!page.records[0].id.is_empty());
    assert!(page.records[0].created > 0);
}

#
fn parse_anthropic_malformed_created_at_yields_zero() {
    //
    //
    //
    //
    let body = br#"{"data":[{"id":"m","created_at":"not-a-date"}]}"#;
    let page = parse_anthropic_models_response(body).unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].created, 0);
}

#
fn parse_google_fixture_strips_models_prefix() {
    let page = parse_google_models_response(&fixture("google.json")).unwrap();
    assert_eq!(page.records.len(), 50);
    for r in &page.records {
        assert!(!r.id.is_empty());
        assert!(!r.id.starts_with("models/"));
    }
    assert!(page.records.iter().any(|r| r.context_window > 0));
}
