//!
//!
//!
//!
//!
//!
//!

use std::io::Write;

use llmkit::builders::openai;

#
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = openai(key);

    //
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
    //
    Ok(())
}
