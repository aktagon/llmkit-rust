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

    let temp_path = std::env::temp_dir().join("llmkit-rust-example-upload.json");
    let data = br#"{"hello":"world"}"#;
    std::fs::write(&temp_path, data)?;

    //
    let file_from_path = c
        .upload()
        .path(temp_path.to_string_lossy().to_string())
        .run()
        .await?;
    println!("path-upload: id={} name={}", file_from_path.id, file_from_path.name);

    //
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
