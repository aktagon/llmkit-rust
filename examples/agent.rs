//! Agent tool loop with a single `add` tool.
//!
//! Run with: `cargo run --example agent`
//!
//! Set OPENAI_API_KEY in the environment for a live call; the example
//! falls back to `sk-test` for offline compilation and the smoke-test
//! suite (`tests/examples.rs`).

use llmkit::builders::openai;
use llmkit::Tool;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = openai(key);

    let add = Tool::new(
        "add",
        "Add two numbers",
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": {"type": "number"},
                "b": {"type": "number"}
            }
        }),
        |args| {
            let a = args["a"].as_f64().ok_or_else(|| "a not a number".to_string())?;
            let b = args["b"].as_f64().ok_or_else(|| "b not a number".to_string())?;
            Ok((a + b).to_string())
        },
    );

    let mut bot = c
        .agent()
        .system("You are a calculator.")
        .add_tool(add)
        .max_tool_iterations(5);

    let resp = bot.prompt("What is 2+3?").await?;
    println!("{}", resp.text);
    Ok(())
}
