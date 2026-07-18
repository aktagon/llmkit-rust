use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, HOST};
use reqwest::Url;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Intermediate signing artifacts. Production callers discard them; the
/// wire-conformance driver (CR-002, tests/sigv4_wire.rs) asserts the
/// canonical request byte-identically against the shared golden.
pub struct SigV4Signature {
    pub canonical_request: String,
    pub string_to_sign: String,
    pub authorization: String,
}

pub(crate) fn sign_request(
    request: &mut reqwest::Request,
    body: &[u8],
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    region: &str,
    service: &str,
) {
    sign_request_at(
        request,
        body,
        access_key,
        secret_key,
        session_token,
        region,
        service,
        Utc::now(),
    );
}

/// `sign_request` with an injected clock (CR-002): the timestamp is the only
/// non-deterministic signing input, so a fixed `now` makes the whole signature
/// chain reproducible for the cross-SDK golden. Conformance-driver seam, not
/// public API.
#[allow(clippy::too_many_arguments)]
pub fn sign_request_at(
    request: &mut reqwest::Request,
    body: &[u8],
    access_key: &str,
    secret_key: &str,
    session_token: &str,
    region: &str,
    service: &str,
    now: DateTime<Utc>,
) -> SigV4Signature {
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

    SigV4Signature {
        canonical_request,
        string_to_sign,
        authorization,
    }
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

#[cfg(test)]
mod tests {
    use super::{canonical_query_string, sha256_hex, sign_request};

    #[test]
    fn sha256_hex_empty() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn canonical_query_string_is_sorted() {
        let url = reqwest::Url::parse("https://example.com/path?b=2&a=1&c=3").expect("url");
        assert_eq!(canonical_query_string(&url), "a=1&b=2&c=3");
    }

    #[test]
    fn sign_request_adds_sigv4_headers() {
        let client = reqwest::Client::new();
        let mut request = client
            .post("https://bedrock-runtime.us-east-1.amazonaws.com/model/test-model/converse")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{\"messages\":[]}".to_string())
            .build()
            .expect("request");

        sign_request(
            &mut request,
            br#"{"messages":[]}"#,
            "AKIAIOSFODNN7EXAMPLE",  // AWS docs example creds #gitleaks:allow
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "",
            "us-east-1",
            "bedrock",
        );

        let authorization = request
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/"));  // AWS docs example creds #gitleaks:allow
        assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
        assert!(authorization.contains("SignedHeaders="));
        assert!(authorization.contains("Signature="));
        assert!(request.headers().contains_key("x-amz-date"));
        assert!(request.headers().contains_key("x-amz-content-sha256"));
    }
}
