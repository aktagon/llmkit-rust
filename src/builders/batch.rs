//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use std::future::{Future, IntoFuture};
use std::pin::Pin;

use crate::error::Error;
use crate::job::JobStatus;
use crate::options::PromptOptions;
use crate::structs::{BatchHandle, Response};
use crate::types::{Provider, Request};

use super::text::{build_options, build_provider, build_request};
use super::Text;

///
///
///
///
#
pub trait BatchHandleExt {
    async fn wait(&self) -> Result<Vec<Response>, Error>;

    ///
    ///
    ///
    ///
    ///
    ///
    ///
    ///
    ///
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

//
//
//
//
impl IntoFuture for BatchHandle {
    type Output = Result<Vec<Response>, Error>;
    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move { self.wait().await })
    }
}

//
//
//
//
//
fn batch_inputs(b: &Text, prompts: &[String]) -> (Provider, Vec<Request>, PromptOptions) {
    let provider = build_provider(b);
    let requests: Vec<Request> = prompts.iter().map(|p| build_request(b, p)).collect();
    let opts = build_options(b);
    (provider, requests, opts)
}

///
///
///
///
///
///
pub(crate) async fn text_batch(b: Text, prompts: Vec<String>) -> Result<BatchHandle, Error> {
    reject_non_default_protocol(&b, "batch")?;
    let (provider, requests, opts) = batch_inputs(&b, &prompts);
    crate::batch::submit_batch(&provider, &requests, opts).await
}

//
//
//
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
