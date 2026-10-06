use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::Error;

/// The `reqwest::Client` for a given idle timeout (BUG-062): one connection
/// pool and one TLS configuration per distinct timeout value, built once and
/// reused. Every request used to build its own (BUG-063), which parsed the
/// root store and built a pool it dropped after one request.
///
/// The timeout is reqwest's `read_timeout`: it bounds the wait for the
/// response headers and every gap between body chunks, and resets after each
/// successful read, so a long healthy stream never times out. Expiry surfaces
/// as `Error::Http` with `is_timeout()` true. `Duration::ZERO` disables it.
pub(crate) fn client_for(timeout: Duration) -> Result<reqwest::Client, Error> {
    static CLIENTS: OnceLock<Mutex<HashMap<Duration, reqwest::Client>>> = OnceLock::new();
    let mut clients = CLIENTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(client) = clients.get(&timeout) {
        return Ok(client.clone());
    }
    let mut builder = reqwest::Client::builder();
    if !timeout.is_zero() {
        builder = builder.read_timeout(timeout);
    }
    let client = builder.build()?;
    clients.insert(timeout, client.clone());
    Ok(client)
}

/// Apply caller custom headers (Client::add_header, ADR-052) to an
/// already-signed SigV4 request. Skips any header whose name already exists
/// (i.e. an AWS-signed header) so the signature is never altered; a gateway
/// in front of Bedrock can still read the extra unsigned headers.
fn apply_unsigned_headers(request: &mut reqwest::Request, custom_headers: &[(String, String)]) {
    for (name, value) in custom_headers {
        if let (Ok(hn), Ok(hv)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            if !request.headers().contains_key(&hn) {
                request.headers_mut().insert(hn, hv);
            }
        }
    }
}

pub async fn post_json(
    url: &str,
    timeout: Duration,
    body: serde_json::Value,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.post(url).json(&body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}

/// POST a JSON body and return the raw response bytes (not text). Used by wire
/// shapes whose response body is binary, e.g. OpenAI /v1/audio/speech returns
/// raw audio bytes (ADR-051).
pub async fn post_json_bytes(
    url: &str,
    timeout: Duration,
    body: serde_json::Value,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, Vec<u8>), Error> {
    let client = client_for(timeout)?;
    let mut request = client.post(url).json(&body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    Ok((status, bytes.to_vec()))
}

pub async fn post_json_sigv4(
    url: &str,
    timeout: Duration,
    body: serde_json::Value,
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    region: &str,
    service: &str,
    custom_headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let body_bytes = serde_json::to_vec(&body)?;
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body_bytes.clone())
        .build()?;
    crate::sigv4::sign_request(
        &mut request,
        &body_bytes,
        access_key,
        secret_key,
        session_token,
        region,
        service,
    );
    apply_unsigned_headers(&mut request, custom_headers);
    let response = client.execute(request).await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}

/// SigV4-signed GET with an empty body (Bedrock async-invoke poll).
/// The url MUST already carry the ARN percent-encoded as a single path
/// segment (`/`→`%2F`, `:` left literal) so the signer's canonical path —
/// derived from `Url::path()`, which preserves the encoding — equals the
/// wire path.
pub async fn get_text_sigv4(
    url: &str,
    timeout: Duration,
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    region: &str,
    service: &str,
    custom_headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.get(url).build()?;
    crate::sigv4::sign_request(
        &mut request,
        b"",
        access_key,
        secret_key,
        session_token,
        region,
        service,
    );
    apply_unsigned_headers(&mut request, custom_headers);
    let response = client.execute(request).await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}

pub async fn get_text(
    url: &str,
    timeout: Duration,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}

pub async fn get_bytes(
    url: &str,
    timeout: Duration,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, Vec<u8>), Error> {
    let client = client_for(timeout)?;
    let mut request = client.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    Ok((status, bytes.to_vec()))
}

pub async fn post_multipart(
    url: &str,
    timeout: Duration,
    form: reqwest::multipart::Form,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.post(url).multipart(form);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}
