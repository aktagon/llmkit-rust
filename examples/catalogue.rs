//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use llmkit::builders::anthropic;
use llmkit::{Capability, Provider, ProviderName};

#
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = anthropic(key.clone());

    //
    let all = c.models().list();
    println!("compiled-in non-empty: {}", !all.is_empty());

    let info = c.models().get("claude-opus-4-7");
    let ctx_pos = info.as_ref().map(|m| m.context_window > 0).unwrap_or(false);
    println!("claude-opus-4-7 context > 0: {ctx_pos}");

    let chat = c.models().with_capability(Capability::ChatCompletion).list();
    println!("chat-capable non-empty: {}", !chat.is_empty());

    //
    let configured: Vec<String> = c
        .providers()
        .list()
        .iter()
        .map(|p| p.slug.to_string())
        .collect();
    println!("configured: [{}]", configured.join(", "));
    println!("supported >= 1: {}", !llmkit::providers::list().is_empty());

    //
    let p = Provider::new(ProviderName::Anthropic, key);
    let live = c.models().live().await;
    println!("live models: {}", live.models.len());

    let scoped = c.models().provider(p.clone()).list().await?;
    println!("scoped list: {}", scoped.len());

    let raw_scoped = c.models().provider(p).raw().list().await?;
    let raw_populated = raw_scoped.first().map(|m| m.raw.is_some()).unwrap_or(false);
    println!("raw populated: {raw_populated}");

    Ok(())
}
