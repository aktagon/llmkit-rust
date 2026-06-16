//! Phase 2.6 catalogue tests (ADR-019). Mirror of Go go/catalogue_test.go,
//! TS ts/tests/catalogue.test.ts, and Python python/tests/test_catalogue.py.

use llmkit::builders::{anthropic, cohere, openai};
use llmkit::providers::generated::providers::ProviderName;
use llmkit::{Capability, CatalogueError, Provider};

#[test]
fn models_list_returns_compiled_in_catalogue() {
    let c = anthropic("test-key");
    let models = c.models().list();
    assert!(!models.is_empty(), "expected non-empty compiled-in catalogue");
    // Sort key is (provider name, id); anthropic sorts first.
    assert_eq!(models[0].provider.name, ProviderName::Anthropic);
}

#[test]
fn models_with_capability_narrows_to_image_generation() {
    let c = openai("test-key");
    let all = c.models().list();
    let image_only = c
        .models()
        .with_capability(Capability::ImageGeneration)
        .list();
    assert!(!image_only.is_empty());
    assert!(image_only.len() < all.len());
    for m in &image_only {
        assert!(m.capabilities.contains(&Capability::ImageGeneration));
    }
}

#[test]
fn models_with_capability_chain_is_immutable() {
    // Rust's ownership transfer is the immutability mechanism: a fresh
    // builder is returned, the original is dropped. Same semantic outcome
    // as Go's struct copy and Python's copy.copy.
    let c = openai("test-key");
    let all = c.models().list();
    let filtered = c
        .models()
        .with_capability(Capability::ImageGeneration)
        .list();
    assert!(all.len() > filtered.len());
}

#[test]
fn models_get_returns_known_model() {
    let c = anthropic("test-key");
    let got = c.models().get("claude-opus-4-7");
    assert!(got.is_some());
    assert_eq!(got.unwrap().id, "claude-opus-4-7");
}

#[test]
fn models_get_returns_none_for_unknown_id() {
    let c = anthropic("test-key");
    assert!(c.models().get("nonexistent-model-xyz").is_none());
}

#[test]
fn providers_list_returns_configured_provider_with_endpoint() {
    let c = anthropic("test-key");
    let got = c.providers().list();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, ProviderName::Anthropic);
    assert_eq!(got[0].slug, "anthropic");
}

#[test]
fn providers_list_empty_for_endpointless_provider() {
    let c = cohere("test-key");
    assert!(c.providers().list().is_empty());
}

#[test]
fn providers_list_static_roster_with_wire_names() {
    // Guards against Python's Issue #9 equivalent: ProviderName must
    // be projected to its wire slug ("anthropic"), not its Debug form
    // ("Anthropic" or "ProviderName::Anthropic"). The static roster of
    // every supported provider is `providers::list()` (ADR-040 PSR-005;
    // the client-scoped `.supported()` was removed).
    let supported = llmkit::providers::list();
    assert!(supported.len() >= 10);
    let names: Vec<&str> = supported.iter().map(|p| p.slug).collect();
    assert!(names.contains(&"anthropic"));
    assert!(names.contains(&"openai"));
    assert!(names.contains(&"google"));
}

#[tokio::test]
async fn scoped_list_returns_not_supported_for_endpointless_provider() {
    let c = cohere("test-key");
    let err = c
        .models()
        .provider(Provider::new(ProviderName::Cohere, "k"))
        .list()
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogueError::NotSupported));
}

#[tokio::test]
async fn scoped_get_keeps_not_supported_for_endpointless_provider() {
    let c2 = cohere("test-key");
    let err2 = c2
        .models()
        .provider(Provider::new(ProviderName::Cohere, "k"))
        .get("any")
        .await
        .unwrap_err();
    assert!(matches!(err2, CatalogueError::NotSupported));
}

#[test]
fn scoped_raw_flips_the_chain_flag_via_ownership_transfer() {
    let c = anthropic("test-key");
    let scoped = c.models().provider(Provider::new(ProviderName::Anthropic, "k"));
    let original_flag = scoped.raw_flag;
    let forked = scoped.raw();
    assert!(!original_flag);
    assert!(forked.raw_flag);
}

#[test]
fn catalogue_error_display_messages() {
    // Exercise the Display impls so coverage sees each variant.
    assert!(CatalogueError::NotSupported.to_string().contains("models endpoint"));
    assert!(CatalogueError::Unavailable("x".into()).to_string().contains("unavailable"));
    assert!(CatalogueError::Scope("x".into()).to_string().contains("scope"));
}
