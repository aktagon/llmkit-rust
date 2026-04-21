use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, HOST};
use reqwest::Url;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub(crate) fn sign_request(
    request: &mut reqwest::Request,
    body: &[u8],
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    region: &str,
    service: &str,
) {
    let now = Utc::now();
    let datestamp = now.format("%Y%m%d").to_string();
    let amzdate = now.format("%Y%m%dT%H%M%SZ").to_string();

    let host = request
        .url()
        .host_str()
        .map(|host| match request.url().port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
        .unwrap_or_default();

    request
        .headers_mut()
        .insert(HOST, HeaderValue::from_str(&host).expect("valid host header"));
    request.headers_mut().insert(
        HeaderName::from_static("x-amz-date"),
        HeaderValue::from_str(&amzdate).expect("valid x-amz-date"),
    );
    if !session_token.is_empty() {
        request.headers_mut().insert(
            HeaderName::from_static("x-amz-security-token"),
            HeaderValue::from_str(session_token).expect("valid session token"),
        );
    }

    let payload_hash = sha256_hex(body);
    request.headers_mut().insert(
        HeaderName::from_static("x-amz-content-sha256"),
        HeaderValue::from_str(&payload_hash).expect("valid payload hash"),
    );

    let (signed_headers, canonical_headers) = build_canonical_headers(request.headers(), &host);
    let canonical_request = [
        request.method().as_str().to_string(),
        canonical_uri(request.url()),
        canonical_query_string(request.url()),
        canonical_headers,
        signed_headers.clone(),
        payload_hash,
    ]
    .join("\n");

    let credential_scope = format!("{datestamp}/{region}/{service}/aws4_request");
    let string_to_sign = [
        "AWS4-HMAC-SHA256".to_string(),
        amzdate,
        credential_scope.clone(),
        sha256_hex(canonical_request.as_bytes()),
    ]
    .join("\n");

    let signing_key = derive_signing_key(secret_key, &datestamp, region, service);
    let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    request.headers_mut().insert(
        reqwest::header::AUTHORIZATION,
        HeaderValue::from_str(&authorization).expect("valid authorization header"),
    );
}

fn derive_signing_key(secret_key: &str, datestamp: &str, region: &str, service: &str) -> Vec<u8> {
    let date_key = hmac_sha256(format!("AWS4{secret_key}").as_bytes(), datestamp.as_bytes());
    let region_key = hmac_sha256(&date_key, region.as_bytes());
    let service_key = hmac_sha256(&region_key, service.as_bytes());
    hmac_sha256(&service_key, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("valid hmac key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn canonical_uri(url: &Url) -> String {
    if url.path().is_empty() {
        "/".into()
    } else {
        url.path().into()
    }
}

fn canonical_query_string(url: &Url) -> String {
    let Some(query) = url.query() else {
        return String::new();
    };
    let mut parts = query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    parts.sort();
    parts.join("&")
}

fn build_canonical_headers(headers: &HeaderMap, host: &str) -> (String, String) {
    let mut canonical = headers
        .iter()
        .filter_map(|(name, value)| {
            let lower = name.as_str().to_ascii_lowercase();
            if lower == "host" || lower == "content-type" || lower.starts_with("x-amz-") {
                Some((
                    lower,
                    value
                        .to_str()
                        .unwrap_or_default()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                ))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    if !canonical.iter().any(|(name, _)| name == "host") {
        canonical.push(("host".into(), host.into()));
    }

    canonical.sort_by(|left, right| left.0.cmp(&right.0));

    let signed_headers = canonical
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_headers = canonical
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect::<String>();
    (signed_headers, canonical_headers)
}





















































