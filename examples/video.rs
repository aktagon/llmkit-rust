//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use llmkit::builders::grok;
use llmkit::builders::VideoHandleExt;

#
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("XAI_API_KEY").unwrap_or_else(|_| "test-key".into());
    let c = grok(key);

    //
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
    //
    Ok(())
}
