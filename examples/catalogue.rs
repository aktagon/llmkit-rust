//! Model catalogue + provider lookup.
//!
//! Demonstrates the `c.models()` / `c.providers()` surface (ADR-019).
//! Three parts:
//!
//! 1. Live list — list() fans out to each configured provider's
//!    /v1/models endpoint. The client caches each provider's list in
//!    memory for one hour; fresh() makes the call again and refills it.
//! 2. Providers namespace — configured (have credentials + a /v1/models
//!    endpoint) and supported (every provider the SDK was built with).
//! 3. Scoped — provider(p).get(id) answers from the cached list when it
//!    holds the id; provider(p).list() lists one provider. raw()
//!    additionally populates ModelInfo.raw with the provider-native record.
//!
//! Run with: `cargo run --example catalogue`

use llmkit::builders::anthropic;
use llmkit::{Capability, Provider, ProviderName};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = anthropic(key.clone());

    // 1. Live list, cached for one hour.
    let live = c.models().list().await;
    println!("live models: {}", live.models.len());

    let chat = c
        .models()
        .with_capability(Capability::ChatCompletion)
        .list()
        .await;
    println!("chat-capable non-empty: {}", !chat.models.is_empty());

    let fresh = c.models().fresh().list().await;
    println!("fresh models: {}", fresh.models.len());

    // 2. Providers namespace.
    let configured: Vec<String> = c
        .providers()
        .list()
        .iter()
        .map(|p| p.slug.to_string())
        .collect();
    println!("configured: [{}]", configured.join(", "));
    println!("supported >= 1: {}", !llmkit::providers::list().is_empty());

    // 3. Scoped to one provider.
    let p = Provider::new(ProviderName::Anthropic, key);
    let info = c
        .models()
        .provider(p.clone())
        .get("claude-opus-4-7")
        .await?;
    println!("claude-opus-4-7 context > 0: {}", info.context_window > 0);

    let scoped = c.models().provider(p.clone()).list().await?;
    println!("scoped list: {}", scoped.len());

    let raw_scoped = c.models().provider(p).raw().list().await?;
    let raw_populated = raw_scoped.first().map(|m| m.raw.is_some()).unwrap_or(false);
    println!("raw populated: {raw_populated}");

    Ok(())
}
