// ADR-038: the `providers` namespace (providers::info / providers::list) is the
// narrow public per-provider metadata access (name/env_var/default_model/
// base_url) — the public replacement for reaching into the now-crate-internal
// ProviderSpec (BUG-012). The import is consumer-style (no `generated`
// segment); a missing re-export fails it at compile time. Values are a
// projection of provider A-Box facts, so this test guards against drift.

use llmkit::providers::{self, ProviderInfo};
use llmkit::ProviderName;
use std::str::FromStr;

#[test]
fn info_projects_anthropic_metadata() {
    let info: &ProviderInfo = providers::info(ProviderName::Anthropic);
    assert_eq!(info.id, ProviderName::Anthropic);
    assert_eq!(info.slug, "anthropic");
    assert_eq!(info.env_var, "ANTHROPIC_API_KEY");
    assert_eq!(info.default_model, "claude-sonnet-4-6");
    assert_eq!(info.base_url, "https://api.anthropic.com");
    assert!(!info.browser_callable);
}

// ADR-035: browser_callable is the coarse per-provider CORS fact — true only for
// a host that serves Access-Control-Allow-Origin for direct browser calls
// (google today), false (needs-proxy) otherwise.
#[test]
fn browser_callable_is_the_cors_fact() {
    assert!(providers::info(ProviderName::Google).browser_callable);
    assert!(!providers::info(ProviderName::Grok).browser_callable);
}

#[test]
fn info_is_total_over_all_providers() {
    // info is total over ProviderName: every provider projects a non-empty
    // slug + env var.
    for &name in llmkit::ALL_PROVIDER_NAMES {
        let info = providers::info(name);
        assert!(!info.slug.is_empty(), "{name:?} projects an empty slug");
        assert!(!info.env_var.is_empty(), "{name:?} projects an empty env var");
    }
}

#[test]
fn list_enumerates_every_provider_sorted_by_slug() {
    let all = providers::list();
    assert_eq!(all.len(), llmkit::ALL_PROVIDER_NAMES.len());
    for pair in all.windows(2) {
        assert!(
            pair[0].slug < pair[1].slug,
            "list() not sorted by slug: {:?} before {:?}",
            pair[0].slug,
            pair[1].slug
        );
    }
}

// === BUG-013: slug <-> ProviderName parse boundary (ADR-040 PSR-003) ===

#[test]
fn provider_name_parses_from_slug_and_constructs_a_client() {
    // The reported BUG-013 path: a caller holding a slug string parses it to
    // the typed identity, then constructs a Client with it.
    let id: ProviderName = "anthropic".parse().unwrap();
    assert_eq!(id, ProviderName::Anthropic);
    let _c = llmkit::builders::new_client(id, "k");

    // FromStr is the same boundary reached explicitly.
    assert_eq!(ProviderName::from_str("openai").unwrap(), ProviderName::OpenAI);
}

#[test]
fn provider_name_round_trips_through_as_str() {
    // as_str() is the forward leg; parse() is the inverse — they round-trip.
    let id = providers::list()[0].id;
    assert_eq!(id.as_str().parse::<ProviderName>().unwrap(), id);
    assert_eq!(ProviderName::Anthropic.as_str(), "anthropic");
    assert_eq!(ProviderName::Anthropic.to_string(), "anthropic");
}

#[test]
fn unknown_slug_is_a_parse_error() {
    let err = "nope".parse::<ProviderName>();
    assert!(err.is_err());
    let err: llmkit::UnknownProviderError = "nope".parse::<ProviderName>().unwrap_err();
    assert_eq!(err.0, "nope");
    assert_eq!(err.to_string(), "unknown provider: \"nope\"");
}
