//! Text-to-image generation against Google Nano Banana.
//!
//! Run with: `cargo run --example image`
//!
//! Writes the first generated image to `out.png` in the current
//! directory. Set GOOGLE_API_KEY for a live call; the example falls
//! back to `test-key` for offline compilation and the smoke-test suite
//! (`tests/examples.rs`).

use llmkit::builders::google;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("GOOGLE_API_KEY").unwrap_or_else(|_| "test-key".into());
    let c = google(key);

    let img = c
        .image()
        .model("gemini-3.1-flash-image-preview")
        .aspect_ratio("16:9")
        .image_size("2K")
        .generate("A nano banana dish, studio lighting")
        .await?;

    if let Some(first) = img.images.first() {
        std::fs::write("out.png", &first.bytes)?;
        println!("wrote {} bytes to out.png ({})", first.bytes.len(), first.mime_type);
    } else {
        println!("no images returned");
    }
    Ok(())
}
