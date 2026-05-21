//!
//!
//!
//!
//!
//!

use crate::structs::{BatchHandle, Response};
use crate::error::Error;
use crate::options::PromptOptions;
use crate::types::{Provider, Request};

use super::text::{build_options, build_provider, build_request};
use super::Text;

///
///
///
///
#[allow(async_fn_in_trait)]
pub trait BatchHandleExt {
    async fn wait(&self) -> Result<Vec<Response>, Error>;
}

impl BatchHandleExt for BatchHandle {
    async fn wait(&self) -> Result<Vec<Response>, Error> {
        crate::batch::wait_batch(self, PromptOptions::new()).await
    }
}

//
//
//
//
//
//
fn batch_inputs(b: &Text, prompts: &[String]) -> (Provider, Vec<Request>, PromptOptions) {
    let provider = build_provider(b);
    let requests: Vec<Request> = prompts
        .iter()
        .map(|p| build_request(b, p))
        .collect();
    let opts = build_options(b);
    (provider, requests, opts)
}

pub(crate) async fn text_batch(b: Text, prompts: Vec<String>) -> Result<Vec<Response>, Error> {
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::batch::prompt_batch(&provider, &requests, opts).await
}

pub(crate) async fn text_submit_batch(
    b: Text,
    prompts: Vec<String>,
) -> Result<BatchHandle, Error> {
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::batch::submit_batch(&provider, &requests, opts).await
}
