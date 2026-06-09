//! Text-to-video generation against xAI's Grok Imagine (ADR-034).
//!
//! Run with: `cargo run --example video`
//!
//! Demonstrates the asynchronous handle: `submit` returns immediately with a
//! `VideoHandle`; `wait` polls until the job completes and returns a temporary
//! xAI-hosted URL (url delivery — the SDK returns a link, it does not download
//! the bytes). Set XAI_API_KEY for a live call; the example falls back to
//! `test-key` for offline compilation and the smoke-test suite
//! (`tests/examples.rs`).

use llmkit::builders::grok;
use llmkit::builders::VideoHandleExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("XAI_API_KEY").unwrap_or_else(|_| "test-key".into());
    let c = grok(key);

    // #region video
    let handle = c
        .video()
        .model("grok-imagine-video")
        .submit("a slow cinematic drone shot flying over snow-capped alpine peaks at golden hour")
        .await?;

    let resp = handle.wait().await?;

    if let Some(first) = resp.videos.first() {
        println!(
            "url={} duration={}s mime={}",
            first.url, first.duration_seconds, first.mime_type
        );
    } else {
        println!("no video returned");
    }
    // #endregion
    Ok(())
}
