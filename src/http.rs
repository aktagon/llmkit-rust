use crate::error::Error;

pub async fn post_json(
    url: &str,
    body: serde_json::Value,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = reqwest::Client::new();
    let mut request = client.post(url).json(&body);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}

pub async fn post_json_sigv4(
    url: &str,
    body: serde_json::Value,
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    region: &str,
    service: &str,
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = reqwest::Client::new();
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
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    region: &str,
    service: &str,
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = reqwest::Client::new();
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
    let response = client.execute(request).await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}

pub async fn get_text(url: &str, headers: &[(String, String)]) -> Result<(reqwest::StatusCode, String), Error> {
    let client = reqwest::Client::new();
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
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, Vec<u8>), Error> {
    let client = reqwest::Client::new();
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
    form: reqwest::multipart::Form,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = reqwest::Client::new();
    let mut request = client.post(url).multipart(form);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let text = response.text().await?;
    Ok((status, text))
}
