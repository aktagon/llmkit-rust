//! Streamed text generation with the trailing-handle pattern.
//!
//! Run with: `cargo run --example streaming`
//!
//! The callback fires for each chunk; the awaited terminal returns the
//! final `Response` carrying token counts. This is the trailing-handle
//! pattern from Go / TS / Python expressed in callback form.

use std::io::Write;

use llmkit::builders::openai;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = openai(key);

    let resp = c
        .text()
        .system("Be brief")
        .stream("Tell me a joke", |chunk| {
            print!("{}", chunk);
            let _ = std::io::stdout().flush();
        })
        .await?;

    println!();
    println!("Usage: {} in / {} out", resp.usage.input, resp.usage.output);
    Ok(())
}
