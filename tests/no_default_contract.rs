// ADR-031 honest no-default contract: local daemons declare no registry
// default — what a daemon serves is runtime inventory — so a missing model
// choice surfaces an instructive validation error instead of guessing a
// model the daemon may not have pulled (the BUG-009 guess-then-404).

use llmkit::builders::new_client;
use llmkit::providers;
use llmkit::{Error, ProviderName};

#[tokio::test]
async fn no_model_on_local_daemon_errors_naming_provider() {
    let mut client = new_client(ProviderName::Ollama, "unused");
    let result = client.text().prompt("hi").await;
    match result {
        Err(Error::Validation { field, message }) => {
            assert_eq!(field, "model");
            assert!(
                message.contains("\"ollama\" declares no default"),
                "message does not name the provider: {message}"
            );
            assert!(message.contains("live()"), "message lacks the live() hint: {message}");
        }
        other => panic!("expected Error::Validation, got {other:?}"),
    }
}

#[test]
fn registry_facts_locals_no_default_clouds_have_one() {
    let locals = [
        ProviderName::Ollama,
        ProviderName::Vllm,
        ProviderName::Llamacpp,
        ProviderName::Lmstudio,
        ProviderName::Jan,
    ];
    for name in locals {
        let info = providers::info(name);
        assert_eq!(info.default_model, "", "{:?} should declare no default", name);
    }
    for name in [ProviderName::Anthropic, ProviderName::OpenAI, ProviderName::Google] {
        let info = providers::info(name);
        assert_ne!(info.default_model, "", "{:?} should declare a default", name);
    }
}
