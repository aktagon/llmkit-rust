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

use crate::error::Error;
use crate::structs::Response;

use super::text::{build_options, build_provider, build_request};
use super::Text;

pub(crate) async fn text_stream(
    b: Text,
    msg: impl Into<String>,
    callback: impl FnMut(&str),
) -> Result<Response, Error> {
    //
    //
    //
    if b.protocol.as_deref().is_some_and(|p| !p.is_empty()) {
        return Err(Error::Validation {
            field: "protocol",
            message: "protocol (e.g. Responses) is only supported on the prompt terminal, not stream (ADR-055)".into(),
        });
    }
    let final_text: String = msg.into();
    let provider = build_provider(&b);
    let request = build_request(&b, &final_text);
    let options = build_options(&b);
    crate::prompt_stream(&provider, &request, options, callback).await
}
