use thiserror::Error;

///
///
///
#
#
pub enum Error {
    ///
    ///
    ///
    #
    Validation {
        field: &'static str,
        message: String,
    },
    #
    Unsupported(String),
    ///
    ///
    ///
    ///
    ///
    ///
    #
    Http(reqwest::Error),
    #
    Json(#[from] serde_json::Error),
    #
    Api {
        provider: String,
        status_code: u16,
        message: String,
    },
    #
    MiddlewareVeto(String),
    ///
    ///
    ///
    ///
    ///
    ///
    #
    PollTimeout { provider: String, id: String },
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Http(e.without_url())
    }
}

impl From<crate::middleware::MiddlewareVeto> for Error {
    fn from(value: crate::middleware::MiddlewareVeto) -> Self {
        Error::MiddlewareVeto(value.to_string())
    }
}

#
mod tests {
    //
    //
    //
    //
    #
    async fn http_error_display_redacts_query_param_key() {
        let secret = "AIzaSyFAKE_SECRET_DO_NOT_LEAK";
        let url = format!("http://127.0.0.1:1/v1/models?key={secret}");

        let reqwest_err = reqwest::get(&url)
            .await
            .expect_err("connection to a closed port must fail");
        assert!(
            reqwest_err.to_string().contains(secret),
            "test premise broken: raw reqwest::Error should still embed the URL/key"
        );

        let err: super::Error = reqwest_err.into();
        let rendered = err.to_string();

        assert!(
            !rendered.contains(secret),
            "Error::Http must redact the URL — got: {rendered}"
        );
        assert!(
            !rendered.contains("key="),
            "Error::Http must not leak the query string at all — got: {rendered}"
        );
    }
}
