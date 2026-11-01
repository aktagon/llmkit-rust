//! Streamed text generation with the trailing-handle pattern.
//!
//! Run with: `cargo run --example streaming`
//!
//! `stream` returns a `TextStream`. Pull each delta with `next().await`;
//! after it returns `None`, `response()` holds the final `Response` with
//! token counts. Go, TS and Python return the same trailing handle.

use std::io::Write;

use llmkit::builders::openai;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = openai(key);

    // #region stream
    let mut stream = c.text().system("Be brief").stream("Tell me a joke");
    while let Some(chunk) = stream.next().await {
        print!("{}", chunk?);
        let _ = std::io::stdout().flush();
    }
    let resp = stream.response().expect("the stream ended");

    println!();
    println!(
        "Usage: {} in / {} out",
        resp.usage
            .input
            .map_or("unreported".to_string(), |v| v.to_string()),
        resp.usage
            .output
            .map_or("unreported".to_string(), |v| v.to_string())
    );
    // #endregion
    Ok(())
}
