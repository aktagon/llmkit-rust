// SigV4 canonical-request wire driver (CR-002): sign the two production-shaped
// Bedrock requests with an injected clock and assert the canonical request,
// string-to-sign, and Authorization header byte-identically against the shared
// golden at codegen/testdata/wire/sigv4/v1/<fixture>.json. The same fixed
// inputs are hard-coded in every SDK's driver; artifacts are dropped at
// target/wire/sigv4/<fixture>/rust.json for the cross-SDK comparator,
// mirroring the telemetry suite (tests/telemetry.rs).

use chrono::{TimeZone, Utc};
use llmkit::sigv4::{sign_request_at, SigV4Signature};

const ACCESS_KEY: &str = "AKIDEXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";  // AWS docs example creds #gitleaks:allow
const SESSION_TOKEN: &str = "IQoJb3JpZ2luX2VjEXAMPLETOKEN";  // AWS docs example creds #gitleaks:allow

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

// The frozen signing clock shared by every SDK driver: 2026-07-18T00:00:00Z.
fn sigv4_wire_now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 18, 0, 0, 0)
        .single()
        .expect("frozen clock")
}

// Writes the SDK artifact for the cross-SDK comparator and asserts each
// signing artifact equals the shared golden's, field by field.
fn assert_sigv4_wire_golden(fixture: &str, sig: &SigV4Signature) {
    let root = repo_root();

    let artifact = serde_json::json!({
        "canonicalRequest": sig.canonical_request,
        "stringToSign": sig.string_to_sign,
        "authorization": sig.authorization,
    });
    let artifact_path = root.join(format!("target/wire/sigv4/{fixture}/rust.json"));
    std::fs::create_dir_all(artifact_path.parent().unwrap()).expect("mkdir artifact dir");
    std::fs::write(
        &artifact_path,
        serde_json::to_string_pretty(&artifact).expect("serialize artifact"),
    )
    .expect("write artifact");

    let golden_path = root.join(format!("codegen/testdata/wire/sigv4/v1/{fixture}.json"));
    let golden: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&golden_path).expect("read golden"))
            .expect("parse golden");
    for (key, actual) in [
        ("canonicalRequest", &sig.canonical_request),
        ("stringToSign", &sig.string_to_sign),
        ("authorization", &sig.authorization),
    ] {
        let want = golden[key].as_str().expect("golden field");
        assert_eq!(
            actual, want,
            "Rust {fixture} {key} differs from shared golden"
        );
    }
}

// Mirrors post_json_sigv4's request assembly (src/http.rs) for the Bedrock
// Converse chat path: POST, Content-Type set before signing (so it joins the
// signed set), session token present, model id ':' literal in the path.
#[test]
fn sigv4_wire_chat_post() {
    let body = br#"{"messages":[{"role":"user","content":[{"text":"Hello, Bedrock"}]}]}"#;
    let client = reqwest::Client::new();
    let mut request = client
        .post("https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-haiku-20240307-v1:0/converse")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body.to_vec())
        .build()
        .expect("build request");

    let sig = sign_request_at(
        &mut request,
        body,
        ACCESS_KEY,
        SECRET_KEY,
        SESSION_TOKEN,
        "us-east-1",
        "bedrock",
        sigv4_wire_now(),
    );
    assert_sigv4_wire_golden("sigv4-chat-post", &sig);
}

// Mirrors get_text_sigv4's request assembly (src/http.rs) for the Bedrock
// async-invoke poll: GET, empty body (empty-string SHA-256 payload hash), no
// Content-Type, no session token, and the invocation ARN percent-encoded as
// ONE path segment ('/' -> %2F, ':' literal) — reqwest's Url::path() preserves
// the encoding, so the signed path equals the wire path (asserted via the
// golden's canonicalRequest).
#[test]
fn sigv4_wire_poll_get() {
    let client = reqwest::Client::new();
    let mut request = client
        .get("https://bedrock-runtime.us-west-2.amazonaws.com/async-invoke/arn:aws:bedrock:us-west-2:123456789012:async-invoke%2Fabc123xyz")
        .build()
        .expect("build request");

    let sig = sign_request_at(
        &mut request,
        b"",
        ACCESS_KEY,
        SECRET_KEY,
        "",
        "us-west-2",
        "bedrock",
        sigv4_wire_now(),
    );
    assert_sigv4_wire_golden("sigv4-poll-get", &sig);
}
