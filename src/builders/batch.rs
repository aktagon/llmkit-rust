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
use crate::middleware::MiddlewareFn;
use crate::options::PromptOptions;
use crate::types::{Provider, Request, Response};

use super::text::{build_provider, build_request};
use super::Text;

/// Extension trait — adds `wait()` to the legacy BatchHandle so the
/// typed-builder API can offer a method-style call site without
/// modifying the legacy struct.
#[allow(async_fn_in_trait)]
pub trait BatchHandleExt {
    async fn wait(&self) -> Result<Vec<Response>, Error>;
}

impl BatchHandleExt for BatchHandle {
    async fn wait(&self) -> Result<Vec<Response>, Error> {
        crate::wait_batch(self, PromptOptions::new()).await
    }
}

fn batch_inputs(b: &Text, prompts: &[String]) -> (Provider, Vec<Request>, PromptOptions) {
    let provider = build_provider(b);
    let requests: Vec<Request> = prompts
        .iter()
        .map(|p| build_request(b, p))
        .collect();
    let mut opts = PromptOptions::new();
    if !b.middleware.is_empty() {
        opts.middleware = b.middleware.iter().cloned().collect::<Vec<MiddlewareFn>>();
    }
    (provider, requests, opts)
}

pub async fn text_batch(b: Text, prompts: Vec<String>) -> Result<Vec<Response>, Error> {
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::prompt_batch(&provider, &requests, opts).await
}

pub async fn text_submit_batch(
    b: Text,
    prompts: Vec<String>,
) -> Result<BatchHandle, Error> {
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::submit_batch(&provider, &requests, opts).await
}
