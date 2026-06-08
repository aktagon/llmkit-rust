//! Prompt caching against Anthropic — reuse a long stable system prefix.
//!
//! Run with: `cargo run --example caching`
//!
//! Set ANTHROPIC_API_KEY for a live call; the example falls back to
//! `sk-test` for offline compilation and the smoke-test suite
//! (`tests/examples.rs`). `.caching()` marks the system prefix as
//! cacheable. Anthropic only caches prefixes above a minimum token
//! threshold, so the system prompt here is intentionally long.

use llmkit::builders::anthropic;

const SYSTEM_PROMPT: &str = "\
You are a meticulous technical editor for a software documentation team.
Always answer in British English. Preserve the author's voice. Never invent
facts. When a passage is ambiguous, flag it rather than guessing. Keep code
blocks verbatim. Prefer active voice and short sentences. Reject marketing
language. When asked for a summary, lead with the single most important point.
This instruction block is stable across the whole editing session and is a good
candidate for prompt caching: it is long, identical on every turn, and never
changes between requests.";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = anthropic(key);

    let resp = c
        .text()
        .system(SYSTEM_PROMPT)
        .caching()
        .prompt("Summarize the editing rules in one sentence.")
        .await?;

    println!("{}", resp.text);
    println!(
        "cache_read={} cache_write={}",
        resp.usage.cache_read, resp.usage.cache_write
    );
    Ok(())
}
