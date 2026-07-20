//!
//!
//!
//!
//!
//!
//!
//!
//!

use llmkit::builders::vertex;

#
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("GOOGLE_API_KEY").unwrap_or_else(|_| "test-key".into());
    let c = vertex(key);

    //
    let resp = c
        .music()
        .model("lyria-002")
        .generate("a calm instrumental, warm piano and soft strings")
        .await?;

    if let Some(first) = resp.audio.first() {
        std::fs::write("out.wav", &first.bytes)?;
        println!("wrote {} bytes to out.wav ({})", first.bytes.len(), first.mime_type);
    } else {
        println!("no audio returned");
    }
    //
    Ok(())
}
