//! Wires the Text builder's `batch` terminal — a text execution mode
//! (parallel to `stream`) that queues one request per prompt and returns
//! a [`BatchHandle`].
//!
//! Note: BatchHandle is an ontology-generated pure-data struct
//! (ADR-018). The `wait()` / `poll()` methods are added via the
//! `BatchHandleExt` trait below so the data struct stays generated while
//! the behavior stays hand-coded. The handle also `impl IntoFuture`
//! (ADR-064 AJU-007) so the blocking one-liner
//! `c.text().batch(...).await?.await?` delegates to `wait`.

use std::future::{Future, IntoFuture};
use std::pin::Pin;

use crate::error::Error;
use crate::job::JobStatus;
use crate::options::PromptOptions;
use crate::structs::{BatchHandle, Response};
use crate::types::{Provider, Request};

use super::text::{build_options, build_provider, build_request};
use super::Text;

/// Extension trait — adds `wait()` to BatchHandle so the typed-builder
/// API can offer a method-style call site. `BatchHandle.raw` (ADR-014)
/// is honored automatically; cross-process resume callers set the
/// field on the struct before calling `wait()`.
#[allow(async_fn_in_trait)]
pub trait BatchHandleExt {
    async fn wait(&self) -> Result<Vec<Response>, Error>;

    /// Performs exactly ONE provider round-trip and returns the normalized
    /// [`JobStatus`] (ADR-063 POLL-001) — the enterprise seam for callers that
    /// drive the poll loop from their own orchestrator instead of blocking on
    /// `wait`. On a completed batch `JobStatus.result` carries the ordered
    /// responses (the two-hop result fetch is performed inline); a
    /// provider-reported terminal failure yields `JobState::Failed` with the
    /// status on `JobStatus.cause`; otherwise `result` is `None` and the state
    /// is `Running`. Honors `self.raw` like `wait`, and is safe on a
    /// reconstituted handle (ADR-014 cross-process resume; POLL-005).
    async fn poll(&self) -> Result<JobStatus<Vec<Response>>, Error>;
}

impl BatchHandleExt for BatchHandle {
    async fn wait(&self) -> Result<Vec<Response>, Error> {
        crate::batch::wait_batch(self, PromptOptions::new(), crate::batch::BatchPoll::default()).await
    }

    async fn poll(&self) -> Result<JobStatus<Vec<Response>>, Error> {
        let adapter = crate::batch::new_batch_adapter(self, self.raw)?;
        crate::job::poll_once(&adapter).await
    }
}

// ADR-064 AJU-007: awaiting a BatchHandle directly delegates to `wait`, so the
// blocking one-liner `c.text().batch(...).await?.await?` works — the reqwest
// RequestBuilder idiom. The compose stays explicit (`batch`, then the await);
// the handle is a durable, re-awaitable value.
impl IntoFuture for BatchHandle {
    type Output = Result<Vec<Response>, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.wait().await })
    }
}

// ADR-012 REQ-PROP-003: every chain field set on the Text builder must
// propagate through batch the same way it propagates through Text::prompt.
// Reusing build_options / build_request (defined in text.rs) keeps the
// per-chain-field translation in one place so the batch wire body is
// semantically identical to a one-shot Text::prompt call with the same chain.
fn batch_inputs(b: &Text, prompts: &[String]) -> (Provider, Vec<Request>, PromptOptions) {
    let provider = build_provider(b);
    let requests: Vec<Request> = prompts.iter().map(|p| build_request(b, p)).collect();
    let opts = build_options(b);
    (provider, requests, opts)
}

/// Queues a batch and returns a [`BatchHandle`] without blocking. The chain's
/// accumulated config (system, schema, model, ...) applies to EVERY prompt in
/// the vector. The chain's `raw()` opt-in (ADR-014) is remembered on the
/// returned handle so `wait`/`poll` honor it without the caller re-specifying.
/// The blocking one-liner is the compose `batch(...).await?` then awaiting the
/// handle (ADR-064 AJU-007).
pub(crate) async fn text_batch(b: Text, prompts: Vec<String>) -> Result<BatchHandle, Error> {
    reject_non_default_protocol(&b, "batch")?;
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::batch::submit_batch(&provider, &requests, opts).await
}

// ADR-055: Protocol (e.g. Responses) is prompt-only in slice 1. The batch
// terminal rejects a non-default protocol loudly rather than silently sending a
// Chat Completions batch (uniform across the four SDKs).
fn reject_non_default_protocol(b: &Text, terminal: &str) -> Result<(), Error> {
    if b.protocol.as_deref().is_some_and(|p| !p.is_empty()) {
        return Err(Error::Validation {
            field: "protocol",
            message: format!(
                "protocol (e.g. Responses) is only supported on the prompt terminal, not {terminal} (ADR-055)"
            ),
        });
    }
    Ok(())
}
