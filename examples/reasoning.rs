//! Reasoning-effort prompting against OpenAI o-series models.
//!
//! Run with: `cargo run --example reasoning`
//!
//! Set OPENAI_API_KEY for a live call; the example falls back to
//! `sk-test` for offline compilation and the smoke-test suite
//! (`tests/examples.rs`). `.reasoning_effort("high")` asks o-series /
//! thinking models to spend more hidden reasoning tokens before
//! answering; `resp.usage.reasoning` reports how many they used.

use llmkit::builders::openai;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = openai(key);

    let resp = c
        .text()
        .reasoning_effort("high")
        .prompt("A bat and a ball cost 1.10 in total. The bat costs 1.00 more than the ball. How much is the ball?")
        .await?;

    println!("{}", resp.text);
    println!("reasoning tokens: {}", resp.usage.reasoning);
    Ok(())
}
