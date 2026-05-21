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
    let final_text: String = msg.into();
    let provider = build_provider(&b);
    let request = build_request(&b, &final_text);
    let options = build_options(&b);
    crate::prompt_stream(&provider, &request, options, callback).await
}
