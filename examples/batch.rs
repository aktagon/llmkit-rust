//! Batch prompting against Anthropic — submit many prompts in one call.
//!
//! Run with: `cargo run --example batch`
//!
//! Set ANTHROPIC_API_KEY for a live call; the example falls back to
//! `sk-test` for offline compilation and the smoke-test suite
//! (`tests/examples.rs`). `batch` queues the batch and returns a
//! handle; awaiting the handle waits until every prompt finishes and
//! returns the responses in submission order.

use llmkit::builders::anthropic;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = anthropic(key);

    let results = c
        .text()
        .system("Be brief")
        .batch(vec![
            "Name a primary color.".to_string(),
            "Name a noble gas.".to_string(),
            "Name a prime number.".to_string(),
        ])
        .await?
        .await?;

    for r in &results {
        println!("{}", r.text);
    }
    Ok(())
}
