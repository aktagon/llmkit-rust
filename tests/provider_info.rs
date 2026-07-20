//
//
//
//
//
//

use llmkit::providers::{self, ProviderInfo};
use llmkit::ProviderName;
use std::str::FromStr;

#
fn info_projects_anthropic_metadata() {
    let info: &ProviderInfo = providers::info(ProviderName::Anthropic);
    assert_eq!(info.id, ProviderName::Anthropic);
    assert_eq!(info.slug, "anthropic");
    assert_eq!(info.env_var, "ANTHROPIC_API_KEY");
    assert_eq!(info.default_model, "claude-sonnet-4-6");
    assert_eq!(info.base_url, "https://api.anthropic.com");
    assert!(!info.browser_callable);
}

//
//
//
//
//
#
fn browser_callable_is_the_cors_fact() {
    assert!(providers::info(ProviderName::Google).browser_callable);
    assert!(!providers::info(ProviderName::OpenAI).browser_callable);
    assert!(!providers::info(ProviderName::Grok).browser_callable);
}

#
fn info_is_total_over_all_providers() {
    //
    //
    for &name in llmkit::ALL_PROVIDER_NAMES {
        let info = providers::info(name);
        assert!(!info.slug.is_empty(), "{name:?} projects an empty slug");
        assert!(!info.env_var.is_empty(), "{name:?} projects an empty env var");
    }
}

#
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

//

#
fn provider_name_parses_from_slug_and_constructs_a_client() {
    //
    //
    let id: ProviderName = "anthropic".parse().unwrap();
    assert_eq!(id, ProviderName::Anthropic);
    let _c = llmkit::builders::new_client(id, "k");

    //
    assert_eq!(ProviderName::from_str("openai").unwrap(), ProviderName::OpenAI);
}

#
fn provider_name_round_trips_through_as_str() {
    //
    let id = providers::list()[0].id;
    assert_eq!(id.as_str().parse::<ProviderName>().unwrap(), id);
    assert_eq!(ProviderName::Anthropic.as_str(), "anthropic");
    assert_eq!(ProviderName::Anthropic.to_string(), "anthropic");
}

#
fn unknown_slug_is_a_parse_error() {
    let err = "nope".parse::<ProviderName>();
    assert!(err.is_err());
    let err: llmkit::UnknownProviderError = "nope".parse::<ProviderName>().unwrap_err();
    assert_eq!(err.0, "nope");
    assert_eq!(err.to_string(), "unknown provider: \"nope\"");
}
