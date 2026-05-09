//! Phase 3 slice 2a — wires Upload::run against legacy `upload_file`.
//!
//! Rust legacy `upload_file(provider, path, middleware)` is path-based
//! (matches Go, inverse of TS). The Bytes branch is deferred — it
//! requires a main-package change adding a bytes-accepting variant.

use crate::error::Error;
use crate::types::{File, Provider};

use super::Upload;

pub async fn upload_run(b: Upload) -> Result<File, Error> {
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
    if has_bytes {
        return Err(Error::Validation {
            field: "Upload",
            message: "bytes() not yet wired (Rust phase 3 follow-up); use path() for now"
                .into(),
        });
    }

    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
    };
    let path = b.path.clone().unwrap_or_default();
    crate::uploads::upload_file(&provider, path, &b.middleware).await
}
