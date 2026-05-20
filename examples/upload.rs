//! Upload a file via Path and Bytes paths against OpenAI's `/v1/files`.
//!
//! Run with: `cargo run --example upload`
//!
//! Writes a temporary file alongside, then uploads it once by path and
//! once by bytes. Set OPENAI_API_KEY for a live call; the example falls
//! back to `sk-test` for offline compilation and the smoke-test suite
//! (`tests/examples.rs`).

use llmkit::builders::openai;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "sk-test".into());
    let c = openai(key);

    let temp_path = std::env::temp_dir().join("llmkit-rust-example-upload.json");
    let data = br#"{"hello":"world"}"#;
    std::fs::write(&temp_path, data)?;

    // Path branch — filename + mime inferred from the path.
    let file_from_path = c
        .upload()
        .path(temp_path.to_string_lossy().to_string())
        .run()
        .await?;
    println!("path-upload: id={} name={}", file_from_path.id, file_from_path.name);

    // Bytes branch — filename required, mime optional.
    let file_from_bytes = c
        .upload()
        .bytes(data.to_vec())
        .filename("report.json")
        .mime_type("application/json")
        .run()
        .await?;
    println!("bytes-upload: id={} name={}", file_from_bytes.id, file_from_bytes.name);

    Ok(())
}
