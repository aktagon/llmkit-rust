//! Each client's in-memory model-list cache (ADR-102 MPT-005). Mirrors
//! go/model_cache.go.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::models_parsers::ParsedModelRecord;

/// How long a provider's live model list stays fresh.
pub(crate) const MODEL_CACHE_TTL: Duration = Duration::from_secs(60 * 60);

/// The clock the cache reads: time on a monotonic clock, so a wall-clock
/// change cannot stretch or cut the TTL. Tests replace it.
pub(crate) type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// Holds each provider's last successful live model list, keyed by
/// provider name, base URL and API key (`cache_key` in `models.rs`). It stores the parsed records, before enrichment, so the
/// chain's `raw` and `with_capability` settings apply on every read. Only
/// the catalogue runtime reads it; sends never do.
pub(crate) struct ModelCache {
    now: Clock,
    entries: Mutex<HashMap<String, (Vec<ParsedModelRecord>, Duration)>>,
}

impl Default for ModelCache {
    fn default() -> Self {
        let start = crate::clock::Instant::now();
        Self::with_clock(Arc::new(move || start.elapsed()))
    }
}

impl ModelCache {
    pub(crate) fn with_clock(now: Clock) -> Self {
        Self {
            now,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// The cached records for `key` when they are younger than
    /// `MODEL_CACHE_TTL`.
    pub(crate) fn get(&self, key: &str) -> Option<Vec<ParsedModelRecord>> {
        let now = (self.now)();
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let (records, fetched_at) = entries.get(key)?;
        if now.saturating_sub(*fetched_at) >= MODEL_CACHE_TTL {
            return None;
        }
        Some(records.clone())
    }

    /// Stores a successful live list. Failed calls never reach it.
    pub(crate) fn put(&self, key: &str, records: Vec<ParsedModelRecord>) {
        let now = (self.now)();
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_string(), (records, now));
    }
}

// ADR-102 MPT-005: hit within one hour, miss after, failure not cached,
// fresh() bypasses and refills, get served from a fresh cached list. The
// tests move an injected clock and count the requests a std::net mock
// server receives. Mirrors go/model_cache_test.go.
#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use super::{ModelCache, MODEL_CACHE_TTL};
    use crate::builders::ScopedModels;
    use crate::builders::{openai, Client};
    use crate::providers::generated::providers::ProviderName;
    use crate::types::Provider;

    const ONE_MODEL: &str = r#"{"object":"list","data":[{"id":"gpt-5","object":"model","created":1715367049,"owned_by":"system"}]}"#;
    const TWO_MODELS: &str = r#"{"object":"list","data":[{"id":"gpt-5","object":"model","created":1715367049,"owned_by":"system"},{"id":"o3","object":"model","created":1715367049,"owned_by":"system"}]}"#;

    /// The OpenAI /v1/models list, plus a point lookup for any
    /// /v1/models/{id}. `fail_next` fails the next list call with a 503.
    #[derive(Default)]
    struct Mock {
        list: String,
        fail_next: bool,
        requests: Vec<String>,
    }

    fn serve(mock: Arc<Mutex<Mock>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.expect("accept");
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).expect("read");
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                let (status, body) = {
                    let mut m = mock.lock().unwrap();
                    m.requests.push(path.clone());
                    if let Some(id) = path.strip_prefix("/v1/models/") {
                        (
                            "200 OK",
                            format!(
                                r#"{{"id":"{id}","object":"model","created":1715367049,"owned_by":"system"}}"#
                            ),
                        )
                    } else if m.fail_next {
                        m.fail_next = false;
                        ("503 Service Unavailable", r#"{"error":"down"}"#.to_string())
                    } else {
                        ("200 OK", m.list.clone())
                    }
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// An OpenAI client pointed at the mock, with its model cache reading
    /// the returned clock (seconds since the epoch).
    fn cached_client(mock: &Arc<Mutex<Mock>>) -> (Client, Arc<AtomicU64>) {
        let mut client = openai("test-key");
        client.provider.base_url = Some(serve(mock.clone()));
        let clock = Arc::new(AtomicU64::new(1_793_523_600)); // 2026-11-01T09:00:00Z
        let reader = clock.clone();
        client.model_cache = Arc::new(ModelCache::with_clock(Arc::new(move || {
            Duration::from_secs(reader.load(Ordering::SeqCst))
        })));
        (client, clock)
    }

    fn mock(list: &str, fail_next: bool) -> Arc<Mutex<Mock>> {
        Arc::new(Mutex::new(Mock {
            list: list.to_string(),
            fail_next,
            requests: Vec::new(),
        }))
    }

    fn openai_target() -> Provider {
        Provider {
            name: ProviderName::OpenAI,
            api_key: String::new(),
            model: None,
            base_url: None,
            headers: std::collections::HashMap::new(),
            timeout: crate::builders::DEFAULT_TIMEOUT,
        }
    }

    fn scope(client: &Client) -> ScopedModels {
        client.models().provider(openai_target())
    }

    fn requests(mock: &Arc<Mutex<Mock>>) -> Vec<String> {
        mock.lock().unwrap().requests.clone()
    }

    fn ids(models: &[crate::structs::ModelInfo]) -> Vec<String> {
        models.iter().map(|m| m.id.clone()).collect()
    }

    #[tokio::test]
    async fn second_list_within_one_hour_is_served_from_cache() {
        let mock = mock(ONE_MODEL, false);
        let (client, clock) = cached_client(&mock);
        scope(&client).list().await.expect("first list");
        clock.fetch_add(59 * 60, Ordering::SeqCst);
        let got = scope(&client).list().await.expect("second list");
        assert_eq!(requests(&mock).len(), 1);
        assert_eq!(ids(&got), vec!["gpt-5"]);
    }

    #[tokio::test]
    async fn entry_expires_after_one_hour() {
        let mock = mock(ONE_MODEL, false);
        let (client, clock) = cached_client(&mock);
        scope(&client).list().await.expect("first list");
        clock.fetch_add(MODEL_CACHE_TTL.as_secs(), Ordering::SeqCst);
        scope(&client).list().await.expect("second list");
        assert_eq!(requests(&mock).len(), 2);
    }

    #[tokio::test]
    async fn failed_call_is_not_cached() {
        let mock = mock(ONE_MODEL, true);
        let (client, _) = cached_client(&mock);
        assert!(scope(&client).list().await.is_err());
        let got = scope(&client).list().await.expect("second list");
        assert_eq!(requests(&mock).len(), 2);
        assert_eq!(ids(&got), vec!["gpt-5"]);
    }

    #[tokio::test]
    async fn fresh_bypasses_the_cache_and_refills_it() {
        let mock = mock(ONE_MODEL, false);
        let (client, _) = cached_client(&mock);
        client.models().list().await;
        mock.lock().unwrap().list = TWO_MODELS.to_string();
        let fresh = client.models().fresh().list().await;
        assert_eq!(requests(&mock).len(), 2);
        assert_eq!(ids(&fresh.models), vec!["gpt-5", "o3"]);
        let cached = client.models().list().await;
        assert_eq!(requests(&mock).len(), 2);
        assert_eq!(ids(&cached.models), vec!["gpt-5", "o3"]);
    }

    #[tokio::test]
    async fn get_answers_from_a_fresh_cached_list_that_holds_the_id() {
        let mock = mock(ONE_MODEL, false);
        let (client, clock) = cached_client(&mock);
        scope(&client).list().await.expect("list");
        let got = scope(&client).get("gpt-5").await.expect("get cached id");
        assert_eq!(got.id, "gpt-5");
        assert_eq!(requests(&mock).len(), 1);
        // An id the cached list does not hold takes the live point lookup.
        scope(&client).get("o3").await.expect("get uncached id");
        // An expired list does not answer either.
        clock.fetch_add(MODEL_CACHE_TTL.as_secs(), Ordering::SeqCst);
        scope(&client).get("gpt-5").await.expect("get after expiry");
        assert_eq!(
            requests(&mock),
            vec!["/v1/models", "/v1/models/o3", "/v1/models/gpt-5"]
        );
    }

    // The cache key is the provider name, the base URL and the API key the
    // call is sent with (owner, 2026-11-01). Clones share the cache, so a
    // clone pointed at another gateway must not read the first one's list.
    #[tokio::test]
    async fn clones_with_another_base_url_miss_the_cache() {
        let first = mock(ONE_MODEL, false);
        let (base, _) = cached_client(&first);
        scope(&base)
            .list()
            .await
            .expect("list on the first gateway");
        let gateway = mock(TWO_MODELS, false);
        let other = base.clone().base_url(serve(gateway.clone()));
        let got = scope(&other)
            .list()
            .await
            .expect("list on the second gateway");
        assert_eq!(requests(&gateway), vec!["/v1/models"]);
        assert_eq!(ids(&got), vec!["gpt-5", "o3"]);
    }

    #[tokio::test]
    async fn fresh_get_skips_the_cache() {
        let mock = mock(ONE_MODEL, false);
        let (client, _) = cached_client(&mock);
        scope(&client).list().await.expect("list");
        client
            .models()
            .fresh()
            .provider(openai_target())
            .get("gpt-5")
            .await
            .expect("fresh get");
        assert_eq!(requests(&mock), vec!["/v1/models", "/v1/models/gpt-5"]);
    }
}
