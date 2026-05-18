//! Phase 3 slice 2a — wires Text::batch + Text::submit_batch.
//!
//! Note: the typed-builder API exposes `BatchHandle` re-exported from
//! `crate::batch`. To give the typed-builder API a `.wait()` method
//! without polluting the legacy struct, the codegen-emitted `Text::*`
//! terminals call `wait_batch(&handle, ...)` indirectly (the user
//! still calls `handle.wait().await` thanks to the `BatchHandleExt`
//! trait below). This avoids modifying the legacy crate-public type.

use crate::batch::BatchHandle;
use crate::error::Error;
use crate::options::PromptOptions;
use crate::types::{Provider, Request, Response};

use super::text::{build_options, build_provider, build_request};
use super::Text;

/// Extension trait — adds `wait()` to BatchHandle so the typed-builder
/// API can offer a method-style call site. `BatchHandle.raw` (ADR-014)
/// is honored automatically; cross-process resume callers set the
/// field on the struct before calling `wait()`.
#[allow(async_fn_in_trait)]
pub trait BatchHandleExt {
    async fn wait(&self) -> Result<Vec<Response>, Error>;
}

impl BatchHandleExt for BatchHandle {
    async fn wait(&self) -> Result<Vec<Response>, Error> {
        crate::batch::wait_batch(self, PromptOptions::new()).await
    }
}

// ADR-012 REQ-PROP-003: every chain field set on the Text builder must
// propagate through Text::batch / submit_batch the same way it
// propagates through Text::prompt. Reusing build_options (defined in
// text.rs) keeps the per-chain-field translation in one place so the
// batch wire body is semantically identical to a one-shot Text::prompt
// call with the same chain. Previously only b.middleware was forwarded.
fn batch_inputs(b: &Text, prompts: &[String]) -> (Provider, Vec<Request>, PromptOptions) {
    let provider = build_provider(b);
    let requests: Vec<Request> = prompts
        .iter()
        .map(|p| build_request(b, p))
        .collect();
    let opts = build_options(b);
    (provider, requests, opts)
}

pub async fn text_batch(b: Text, prompts: Vec<String>) -> Result<Vec<Response>, Error> {
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::batch::prompt_batch(&provider, &requests, opts).await
}

pub async fn text_submit_batch(
    b: Text,
    prompts: Vec<String>,
) -> Result<BatchHandle, Error> {
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::batch::submit_batch(&provider, &requests, opts).await
}
