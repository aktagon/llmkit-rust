//! Wires `*Upload.run()` against the internal `upload_file` /
//! `upload_bytes` helpers in `crate::uploads`. Both Path and Bytes are
//! wired:
//!
//! - `c.upload().path(p).run().await` — reads `p` from disk; the
//!   multipart filename defaults to `p.file_name()` unless `filename()`
//!   overrides.
//! - `c.upload().bytes(b).filename(n).run().await` — uploads `b`
//!   directly with `n` as the multipart filename.
//!
//! `mime_type()` overrides the filename-extension–based detection in
//! either branch.

use crate::error::Error;
use crate::structs::File;
use crate::types::Provider;

use super::Upload;

pub(crate) async fn upload_run(b: Upload) -> Result<File, Error> {
    let has_bytes = !b.bytes.is_empty();
    let has_path = b.path.as_ref().map(|p| !p.is_empty()).unwrap_or(false);

    if !has_bytes && !has_path {
        return Err(Error::Validation {
            field: "Upload",
            message: "exactly one of bytes() or path() must be set".into(),
        });
    }
    if has_bytes && has_path {
        return Err(Error::Validation {
            field: "Upload",
            message: "bytes() and path() are mutually exclusive".into(),
        });
    }
    if has_bytes && b.filename.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
        return Err(Error::Validation {
            field: "Upload",
            message: "filename() is required when bytes() is set".into(),
        });
    }

    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
    };

    if has_bytes {
        let filename = b.filename.clone().unwrap_or_default();
        let mime_type = b.mime_type.clone().unwrap_or_default();
        return crate::uploads::upload_bytes(&provider, b.bytes, filename, mime_type, &b.middleware)
            .await;
    }

    // Path branch — fall through to the path-based legacy helper. The
    // chained filename()/mime_type() are a slight semantic gap: the
    // legacy upload_file derives both from the path. Wiring overrides
    // through is a tiny refactor (mirror Go) but adds no value for
    // typical callers, so we route them via upload_bytes when set.
    let path = b.path.clone().unwrap_or_default();
    if b.filename.is_some() || b.mime_type.is_some() {
        let data = std::fs::read(&path).map_err(|error| Error::Unsupported(error.to_string()))?;
        let filename = b
            .filename
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                std::path::Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("upload.bin")
                    .to_string()
            });
        let mime_type = b.mime_type.clone().unwrap_or_default();
        return crate::uploads::upload_bytes(&provider, data, filename, mime_type, &b.middleware)
            .await;
    }
    crate::uploads::upload_file(&provider, path, &b.middleware).await
}
