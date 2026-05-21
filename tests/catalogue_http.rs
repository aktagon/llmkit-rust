//! HTTP runtime tests for the catalogue (ADR-019 Phase 3).
//!
//! Mirrors go/models_test.go, ts/tests/catalogue_http.test.ts and
//! python/tests/test_catalogue_http.py. Uses a TcpListener-backed
//! hand-rolled HTTP responder so we don't take a new dev-dep for one
//! test file — the same pattern rust/tests/prompt.rs already runs on.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use llmkit::builders::{anthropic, cohere, google, openai};
use llmkit::providers::generated::providers::ProviderName;
use llmkit::{CatalogueError, Provider};

struct Recorded {
    path: String,
    query: String,
    headers: std::collections::HashMap<String, String>,
}

fn read_request(stream: &mut std::net::TcpStream) -> Recorded {
    let mut buf = [0u8; 4096];
    let mut accumulated = Vec::new();
    loop {
        let n = stream.read(&mut buf).expect("read");
        accumulated.extend_from_slice(&buf[..n]);
        if accumulated.windows(4).any(|w| w == b"\r\n\r\n") || n == 0 {
            break;
        }
    }
    let request = String::from_utf8_lossy(&accumulated).to_string();
    let first_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let path_with_query = parts.get(1).copied().unwrap_or("/");
    let (path, query) = match path_with_query.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (path_with_query.to_string(), String::new()),
    };
    let mut headers = std::collections::HashMap::new();
    for line in request.lines().skip(1) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    Recorded { path, query, headers }
}

/// Start a TCP responder that runs the supplied `handler` per accepted
/// request. The handler returns `(status, body)`. Calls land in
/// `recorded` (the test inspects this to assert pagination cursors,
/// headers, etc.). Returns the mock-server URL the Client should be
/// pointed at via `with_base_url`.
fn start_mock<F>(handler: F, recorded: Arc<Mutex<Vec<Recorded>>>) -> String
where
    F: Fn(&Recorded) -> (u16, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handler = Arc::new(handler);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let req = read_request(&mut stream);
            let (status, body) = handler(&req);
            recorded.lock().unwrap().push(req);
            let resp = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn scoped_list_anthropic_cursor_pagination() {
    let page1 = r#"{"data":[{"type":"model","id":"claude-opus-4-7","display_name":"Claude Opus 4.7","created_at":"2026-04-14T00:00:00Z","max_input_tokens":1000000,"max_tokens":128000},{"type":"model","id":"claude-sonnet-4-6","display_name":"Claude Sonnet 4.6","created_at":"2026-04-14T00:00:00Z","max_input_tokens":1000000,"max_tokens":128000}],"has_more":true,"last_id":"claude-sonnet-4-6"}"#.to_string();
    let page2 = r#"{"data":[{"type":"model","id":"claude-haiku-4-5-20251001","display_name":"Claude Haiku 4.5","created_at":"2026-04-14T00:00:00Z","max_input_tokens":200000,"max_tokens":64000}],"has_more":false,"last_id":"claude-haiku-4-5-20251001"}"#.to_string();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(
        move |req| {
            if req.headers.get("x-api-key").map(String::as_str) != Some("test-key") {
                return (401, "{}".to_string());
            }
            if req.query.contains("after_id=claude-sonnet-4-6") {
                return (200, page2.clone());
            }
            (200, page1.clone())
        },
        recorded.clone(),
    );

    let c = anthropic("test-key").with_base_url(&base);
    let models = c
        .models()
        .provider(Provider::new(ProviderName::Anthropic, "test-key"))
        .list()
        .await
        .expect("list ok");
    assert_eq!(models.len(), 3);
    let recs = recorded.lock().unwrap();
    assert_eq!(recs.len(), 2);
    assert!(recs[1].query.contains("after_id=claude-sonnet-4-6"));
    let opus = models.iter().find(|m| m.id == "claude-opus-4-7").unwrap();
    assert!(!opus.capabilities.is_empty(), "ontology-enriched");
}

#[tokio::test]
async fn scoped_list_google_opaque_token_pagination() {
    let page1 = r#"{"models":[{"name":"models/gemini-2.5-flash","displayName":"Gemini 2.5 Flash","description":"Stable","inputTokenLimit":1048576,"outputTokenLimit":65536}],"nextPageToken":"opaque-cursor-xyz"}"#.to_string();
    let page2 = r#"{"models":[{"name":"models/gemini-2.5-pro","displayName":"Gemini 2.5 Pro","description":"Stable","inputTokenLimit":1048576,"outputTokenLimit":65536}]}"#.to_string();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(
        move |req| {
            if !req.query.contains("key=test-key") {
                return (401, "{}".to_string());
            }
            if req.query.contains("pageToken=opaque-cursor-xyz") {
                return (200, page2.clone());
            }
            (200, page1.clone())
        },
        recorded.clone(),
    );

    let c = google("test-key").with_base_url(&base);
    let models = c
        .models()
        .provider(Provider::new(ProviderName::Google, "test-key"))
        .list()
        .await
        .expect("list ok");
    assert_eq!(models.len(), 2);
    // Parser strips models/ prefix from response.name.
    assert_eq!(models[0].id, "gemini-2.5-flash");
}

#[tokio::test]
async fn scoped_list_openai_non_paginated() {
    let body = r#"{"object":"list","data":[{"id":"gpt-5","object":"model","created":1715367049,"owned_by":"system"},{"id":"gpt-4o","object":"model","created":1715367049,"owned_by":"system"}]}"#.to_string();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(
        move |req| {
            if req.headers.get("authorization").map(String::as_str) != Some("Bearer test-key") {
                return (401, "{}".to_string());
            }
            if !req.query.is_empty() {
                return (400, "{}".to_string());
            }
            (200, body.clone())
        },
        recorded.clone(),
    );

    let c = openai("test-key").with_base_url(&base);
    let models = c
        .models()
        .provider(Provider::new(ProviderName::OpenAI, "test-key"))
        .list()
        .await
        .expect("list ok");
    assert_eq!(recorded.lock().unwrap().len(), 1);
    assert_eq!(models.len(), 2);
}

#[tokio::test]
async fn scoped_list_403_scope_maps_to_scope_sentinel() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(
        move |_| (403, r#"{"error":{"message":"Missing scopes: api.model.read"}}"#.to_string()),
        recorded,
    );
    let c = openai("test-key").with_base_url(&base);
    let err = c
        .models()
        .provider(Provider::new(ProviderName::OpenAI, "test-key"))
        .list()
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogueError::Scope(_)));
}

#[tokio::test]
async fn scoped_list_503_maps_to_unavailable_sentinel() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(move |_| (503, r#"{"error":"down"}"#.to_string()), recorded);
    let c = anthropic("test-key").with_base_url(&base);
    let err = c
        .models()
        .provider(Provider::new(ProviderName::Anthropic, "test-key"))
        .list()
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogueError::Unavailable(_)));
}

#[tokio::test]
async fn scoped_list_endpointless_provider_keeps_not_supported() {
    // No mock needed — runtime short-circuits before any HTTP call.
    let c = cohere("test-key");
    let err = c
        .models()
        .provider(Provider::new(ProviderName::Cohere, "k"))
        .list()
        .await
        .unwrap_err();
    assert!(matches!(err, CatalogueError::NotSupported));
}

#[tokio::test]
async fn scoped_get_anthropic_single_record() {
    let body = r#"{"type":"model","id":"claude-opus-4-7","display_name":"Claude Opus 4.7","created_at":"2026-04-14T00:00:00Z","max_input_tokens":1000000,"max_tokens":128000}"#.to_string();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(
        move |req| {
            if req.path != "/v1/models/claude-opus-4-7" {
                return (404, "{}".to_string());
            }
            (200, body.clone())
        },
        recorded.clone(),
    );
    let c = anthropic("test-key").with_base_url(&base);
    let m = c
        .models()
        .provider(Provider::new(ProviderName::Anthropic, "test-key"))
        .get("claude-opus-4-7")
        .await
        .expect("get ok");
    assert_eq!(m.id, "claude-opus-4-7");
    assert!(!m.capabilities.is_empty());
}

#[tokio::test]
async fn models_live_partial_success_typed_provider_error() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let base = start_mock(move |_| (503, "{}".to_string()), recorded);
    let c = openai("test-key").with_base_url(&base);
    let res = c.models().live().await;
    assert!(res.models.is_empty());
    let err = res.errors.get("openai").expect("openai err present");
    assert_eq!(err.kind, "unavailable");
}
