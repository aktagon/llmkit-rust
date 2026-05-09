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
use crate::types::Response;

use super::text::{build_options, build_provider, build_request};
use super::Text;

pub async fn text_stream(
    b: Text,
    msg: impl Into<String>,
    callback: impl FnMut(&str),
) -> Result<Response, Error> {
    let final_text: String = msg.into();
    let provider = build_provider(&b);
    let request = build_request(&b, &final_text);
    let options = build_options(&b);
    crate::prompt_stream_internal(&provider, &request, options, callback).await
}
