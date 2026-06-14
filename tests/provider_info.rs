// ADR-038: the `providers` namespace (providers::info / providers::list) is the
// narrow public per-provider metadata access (name/env_var/default_model/
// base_url) — the public replacement for reaching into the now-crate-internal
// ProviderSpec (BUG-012). The import is consumer-style (no `generated`
// segment); a missing re-export fails it at compile time. Values are a
// projection of provider A-Box facts, so this test guards against drift.

use llmkit::providers::{self, ProviderInfo};
use llmkit::ProviderName;

#[test]
fn info_projects_anthropic_metadata() {
    let info: &ProviderInfo = providers::info(ProviderName::Anthropic);
    assert_eq!(info.name, "anthropic");
    assert_eq!(info.env_var, "ANTHROPIC_API_KEY");
    assert_eq!(info.default_model, "claude-sonnet-4-6");
    assert_eq!(info.base_url, "https://api.anthropic.com");
}

#[test]
fn info_is_total_over_all_providers() {
    // info is total over ProviderName: every provider projects a non-empty
    // slug + env var.
    for &name in llmkit::ALL_PROVIDER_NAMES {
        let info = providers::info(name);
        assert_eq!(info.name.is_empty(), false, "{name:?} projects an empty slug");
        assert_eq!(info.env_var.is_empty(), false, "{name:?} projects an empty env var");
    }
}

#[test]
fn list_enumerates_every_provider_sorted_by_name() {
    let all = providers::list();
    assert_eq!(all.len(), llmkit::ALL_PROVIDER_NAMES.len());
    for pair in all.windows(2) {
        assert!(
            pair[0].name < pair[1].name,
            "list() not sorted by name: {:?} before {:?}",
            pair[0].name,
            pair[1].name
        );
    }
}
