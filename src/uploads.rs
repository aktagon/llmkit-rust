use std::path::Path;

use serde_json::Value;

use crate::error::Error;
use crate::http::post_multipart;
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::request::file_upload_config;
use crate::request::build_auth_headers;
use crate::types::{File, Provider};

pub async fn upload_file(provider: &Provider, path: impl AsRef<Path>) -> Result<File, Error> {
    let config = provider_config(provider.name);
    let upload = file_upload_config(provider.name).ok_or_else(|| Error::Validation {
        field: "provider",
        message: format!("file upload not supported: {:?}", provider.name),
    })?;

    let path = path.as_ref();
    let data = std::fs::read(path).map_err(|error| Error::Unsupported(error.to_string()))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("upload.bin")
        .to_string();
    let mime_type = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();

    let base = provider
        .base_url
        .clone()
        .unwrap_or_else(|| config.base_url.to_string());
    let mut url = format!("{base}{}", upload.endpoint);
    if !config.auth_query_param.is_empty() && matches!(crate::auth_scheme(provider.name), crate::AuthScheme::QueryParamKey) {
        let separator = if url.contains('?') { "&" } else { "?" };
        url.push_str(separator);
        url.push_str(config.auth_query_param);
        url.push('=');
        url.push_str(&provider.api_key);
    }

    let mut headers = build_auth_headers(provider, config);
    if !upload.beta_header.is_empty() {
        headers.push(("anthropic-beta".into(), upload.beta_header.into()));
    }

    let mut form = reqwest::multipart::Form::new().part(
        upload.field_name.to_string(),
        reqwest::multipart::Part::bytes(data)
            .file_name(filename.clone())
            .mime_str(&mime_type)
            .map_err(|error| Error::Unsupported(error.to_string()))?,
    );

    if !upload.extra_fields_json.is_empty() {
        if let Ok(Value::Object(fields)) = serde_json::from_str::<Value>(upload.extra_fields_json) {
            for (key, value) in fields {
                if let Some(text) = value.as_str() {
                    form = form.text(key, text.to_string());
                }
            }
        }
    }

    if config.system_placement == "SiblingObject" {
        form = form.text(
            "metadata",
            serde_json::json!({"file": {"display_name": filename}})
                .to_string(),
        );
        headers.push(("X-Goog-Upload-Protocol".into(), "multipart".into()));
    }

    let (status, response_body) = post_multipart(&url, form, &headers).await?;
    if !status.is_success() {
        return Err(crate::response::parse_api_error(
            provider,
            status.as_u16(),
            &response_body,
        ));
    }
    let parsed: Value = serde_json::from_str(&response_body)?;

    let mut file = File {
        mime_type,
        name: filename,
        ..File::default()
    };
    if !upload.response_id_path.is_empty() {
        file.id = crate::paths::extract_string_path(&parsed, upload.response_id_path);
    }
    if !upload.response_uri_path.is_empty() {
        file.uri = crate::paths::extract_string_path(&parsed, upload.response_uri_path);
    }
    if !upload.response_name_path.is_empty() {
        file.name = crate::paths::extract_string_path(&parsed, upload.response_name_path);
    }
    if !upload.response_mime_path.is_empty() {
        file.mime_type = crate::paths::extract_string_path(&parsed, upload.response_mime_path);
    }
    Ok(file)
}
