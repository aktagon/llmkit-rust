//!
//!
//!
//!
//!
//!
//!

use llmkit::builders::anthropic;

#
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = anthropic(key);

    let resp = c
        .text()
        .system("Be concise.")
        .temperature(0.3)
        .prompt("Why is the sky blue?")
        .await?;

    println!("{}", resp.text);
    println!("{} input tokens", resp.usage.input);
    println!("{} output tokens", resp.usage.output);
    Ok(())
}
