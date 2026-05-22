//! Spend-cap middleware.
//!
//! Vetoes LLM requests once cumulative cost exceeds a daily budget, and
//! accumulates actual cost on post-phase using a caller-supplied price
//! table. Price tables drift monthly and belong in user code, not the
//! library.
//!
//! Run with: `cargo run --example middleware`
//!
//! Set ANTHROPIC_API_KEY in the environment for a live call; the
//! example falls back to `sk-test` so it still compiles and runs in
//! the smoke-test suite (`tests/examples.rs`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use llmkit::builders::anthropic;
use llmkit::{Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase};

#[derive(Clone, Copy)]
struct Price {
    input: f64,
    output: f64,
}

struct SpendCap {
    budget: f64,
    spent: Mutex<f64>,
    prices: HashMap<String, Price>,
}

impl SpendCap {
    fn new(budget: f64, prices: HashMap<String, Price>) -> Self {
        Self {
            budget,
            spent: Mutex::new(0.0),
            prices,
        }
    }

    fn spent(&self) -> f64 {
        *self.spent.lock().unwrap()
    }

    fn middleware(self: &Arc<Self>) -> MiddlewareFn {
        let s = Arc::clone(self);
        Arc::new(move |e: &Event| {
            if !matches!(e.op, MiddlewareOp::LlmRequest) {
                return None;
            }
            let mut spent = s.spent.lock().unwrap();
            if matches!(e.phase, MiddlewarePhase::Pre) {
                if *spent >= s.budget {
                    return Some(
                        format!(
                            "daily budget ${:.2} exceeded (spent ${:.4})",
                            s.budget, *spent,
                        )
                        .into(),
                    );
                }
                return None;
            }
            if let (Some(p), Some(u)) = (s.prices.get(&e.model), e.usage.as_ref()) {
                *spent += (u.input as f64) * p.input / 1e6
                    + (u.output as f64) * p.output / 1e6;
            }
            None
        })
    }
}

fn token_logger() -> MiddlewareFn {
    Arc::new(|e: &Event| {
        if matches!(e.op, MiddlewareOp::LlmRequest)
            && matches!(e.phase, MiddlewarePhase::Post)
        {
            if let Some(u) = e.usage.as_ref() {
                let secs = e
                    .duration
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                println!(
                    "[{}/{}] in={} out={} cache_read={} took={:.3}s",
                    e.provider, e.model, u.input, u.output, u.cache_read, secs,
                );
            }
        }
        None
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = anthropic(key);

    let mut prices = HashMap::new();
    prices.insert(
        "claude-sonnet-4-5-20250929".to_string(),
        Price {
            input: 3.0,
            output: 15.0,
        },
    );
    let cap = Arc::new(SpendCap::new(5.0, prices));

    let resp = c
        .text()
        .add_middleware(vec![cap.middleware(), token_logger()])
        .prompt("What is 2+2? Reply in one word.")
        .await?;

    println!("Answer: {}", resp.text);
    println!("Spent so far: ${:.4} / $5.00", cap.spent());
    Ok(())
}
