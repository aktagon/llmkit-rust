//!
//!
//!
//!
//!
//!
//!
//!
//!

use llmkit::builders::openai;

#
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
