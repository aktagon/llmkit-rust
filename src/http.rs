use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::Error;
use crate::json_text::write_json;

/// The `reqwest::Client` for a given idle timeout (BUG-062): one connection
/// pool and one TLS configuration per distinct timeout value, built once and
/// reused. Every request used to build its own (BUG-063), which parsed the
/// root store and built a pool it dropped after one request.
///
/// The timeout is an idle timeout: it bounds the wait for the response
/// headers and every gap between body chunks, and resets after each
/// successful read, so a long healthy stream never times out. Expiry returns
/// `Error::Timeout`. `Duration::ZERO` disables it.
///
/// Native builds use reqwest's `read_timeout`. reqwest's wasm32 client has no
/// `read_timeout`, so on wasm32 [`within`] races each wait against
/// `crate::clock::sleep` (BUG-113).
pub(crate) fn client_for(timeout: Duration) -> Result<reqwest::Client, Error> {
    static CLIENTS: OnceLock<Mutex<HashMap<Duration, reqwest::Client>>> = OnceLock::new();
    let mut clients = CLIENTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(client) = clients.get(&timeout) {
        return Ok(client.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let builder = if timeout.is_zero() {
        reqwest::Client::builder()
    } else {
        reqwest::Client::builder().read_timeout(timeout)
    };
    #[cfg(target_arch = "wasm32")]
    let builder = reqwest::Client::builder();
    let client = builder.build()?;
    clients.insert(timeout, client.clone());
    Ok(client)
}

/// Awaits `work`: sending a request, or reading the next body chunk.
///
/// Native builds: reqwest's `read_timeout` already bounds every read, so this
/// only awaits `work`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) async fn within<T>(
    _timeout: Duration,
    work: impl Future<Output = Result<T, reqwest::Error>>,
) -> Result<T, Error> {
    Ok(work.await?)
}

/// Awaits `work`: sending a request, or reading the next body chunk.
///
/// wasm32: fails with `Error::Timeout` when `work` takes longer than
/// `timeout`. A Cloudflare Worker sets no time limit on a subrequest, so
/// without this a silent provider holds the request open without end. On
/// expiry `work` is dropped, and dropping reqwest's request or body drops its
/// `AbortGuard`, which aborts the fetch.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn within<T>(
    timeout: Duration,
    work: impl Future<Output = Result<T, reqwest::Error>>,
) -> Result<T, Error> {
    use std::task::Poll;
    if timeout.is_zero() {
        return Ok(work.await?);
    }
    let mut work = std::pin::pin!(work);
    let mut timer = std::pin::pin!(crate::clock::sleep(timeout));
    std::future::poll_fn(|cx| {
        if let Poll::Ready(result) = work.as_mut().poll(cx) {
            return Poll::Ready(result.map_err(Error::from));
        }
        match timer.as_mut().poll(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Err(Error::Timeout)),
            Poll::Ready(Err(cause)) => Poll::Ready(Err(cause)),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

/// A response body read chunk by chunk, each read under the idle timeout.
pub(crate) struct Body {
    timeout: Duration,
    #[cfg(not(target_arch = "wasm32"))]
    response: reqwest::Response,
    #[cfg(target_arch = "wasm32")]
    chunks: std::pin::Pin<Box<dyn futures_util::Stream<Item = reqwest::Result<Vec<u8>>>>>,
}

impl Body {
    pub(crate) fn new(response: reqwest::Response, timeout: Duration) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Body { timeout, response };
        #[cfg(target_arch = "wasm32")]
        {
            use futures_util::StreamExt;
            let chunks = response
                .bytes_stream()
                .map(|chunk| chunk.map(|bytes| bytes.to_vec()));
            Body {
                timeout,
                chunks: Box::pin(chunks),
            }
        }
    }

    /// The next chunk, or `None` at the end of the body.
    pub(crate) async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, Error> {
        #[cfg(not(target_arch = "wasm32"))]
        return Ok(within(self.timeout, self.response.chunk())
            .await?
            .map(|bytes| bytes.to_vec()));
        #[cfg(target_arch = "wasm32")]
        {
            use futures_util::StreamExt;
            within(self.timeout, async { self.chunks.next().await.transpose() }).await
        }
    }
}

/// The whole body as text. Native builds decode it by the response charset,
/// as reqwest does. wasm32 reads it chunk by chunk under the idle timeout and
/// decodes it as UTF-8, invalid bytes as U+FFFD, as the browser's
/// `Response.text()` does.
pub(crate) async fn read_text(
    response: reqwest::Response,
    timeout: Duration,
) -> Result<String, Error> {
    #[cfg(not(target_arch = "wasm32"))]
    return within(timeout, response.text()).await;
    #[cfg(target_arch = "wasm32")]
    {
        let bytes = read_bytes(response, timeout).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// The whole body as bytes, read chunk by chunk under the idle timeout.
pub(crate) async fn read_bytes(
    response: reqwest::Response,
    timeout: Duration,
) -> Result<Vec<u8>, Error> {
    let mut body = Body::new(response, timeout);
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next_chunk().await? {
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Sends a request and reads its reply as text, both under the idle timeout.
pub(crate) async fn text_reply(
    timeout: Duration,
    send: impl Future<Output = Result<reqwest::Response, reqwest::Error>>,
) -> Result<(reqwest::StatusCode, String), Error> {
    let response = within(timeout, send).await?;
    let status = response.status();
    Ok((status, read_text(response, timeout).await?))
}

/// Sends a request and reads its reply as bytes, both under the idle timeout.
async fn bytes_reply(
    timeout: Duration,
    send: impl Future<Output = Result<reqwest::Response, reqwest::Error>>,
) -> Result<(reqwest::StatusCode, Vec<u8>), Error> {
    let response = within(timeout, send).await?;
    let status = response.status();
    Ok((status, read_bytes(response, timeout).await?))
}

/// Apply caller custom headers (Client::add_header, ADR-052) to an
/// already-signed SigV4 request. Skips any header whose name already exists
/// (i.e. an AWS-signed header) so the signature is never altered; a gateway
/// in front of Bedrock can still read the extra unsigned headers.
fn apply_unsigned_headers(request: &mut reqwest::Request, custom_headers: &[(String, String)]) {
    for (name, value) in custom_headers {
        if let (Ok(hn), Ok(hv)) = (
            reqwest::header::HeaderName::from_bytes(name.as_bytes()),
            reqwest::header::HeaderValue::from_str(value),
        ) {
            if !request.headers().contains_key(&hn) {
                request.headers_mut().insert(hn, hv);
            }
        }
    }
}

pub async fn post_json(
    url: &str,
    timeout: Duration,
    body: serde_json::Value,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = json_post(&client, url, &body)?;
    for (name, value) in headers {
        request = request.header(name, value);
    }
    text_reply(timeout, request.send()).await
}

/// A POST of `body` as JSON, written by [`write_json`], the one body writer
/// (HANDOFF-105 F7): it splices a tool call's argument text into the body.
pub(crate) fn json_post(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<reqwest::RequestBuilder, Error> {
    Ok(client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(write_json(body)?))
}

/// POST with an empty body (no Content-Type), as Google's long-running
/// operation `:cancel` requires.
pub(crate) async fn post_empty(
    url: &str,
    timeout: Duration,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.post(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    text_reply(timeout, request.send()).await
}

/// POST a JSON body and return the raw response bytes (not text). Used by wire
/// shapes whose response body is binary, e.g. OpenAI /v1/audio/speech returns
/// raw audio bytes (ADR-051).
pub async fn post_json_bytes(
    url: &str,
    timeout: Duration,
    body: serde_json::Value,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, Vec<u8>), Error> {
    let client = client_for(timeout)?;
    let mut request = json_post(&client, url, &body)?;
    for (name, value) in headers {
        request = request.header(name, value);
    }
    bytes_reply(timeout, request.send()).await
}

/// The AWS SigV4 signing inputs. Every signed request needs all five.
pub(crate) struct SigV4Credentials<'a> {
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub session_token: &'a str,
    pub region: &'a str,
    pub service: &'a str,
}

pub(crate) async fn post_json_sigv4(
    url: &str,
    timeout: Duration,
    body: serde_json::Value,
    creds: &SigV4Credentials<'_>,
    custom_headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let body_bytes = write_json(&body)?.into_bytes();
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body_bytes.clone())
        .build()?;
    crate::sigv4::sign_request(
        &mut request,
        &body_bytes,
        creds.access_key,
        creds.secret_key,
        creds.session_token,
        creds.region,
        creds.service,
    );
    apply_unsigned_headers(&mut request, custom_headers);
    text_reply(timeout, client.execute(request)).await
}

/// SigV4-signed GET with an empty body (Bedrock async-invoke poll).
/// The url MUST already carry the ARN percent-encoded as a single path
/// segment (`/`→`%2F`, `:` left literal) so the signer's canonical path —
/// derived from `Url::path()`, which preserves the encoding — equals the
/// wire path.
pub(crate) async fn get_text_sigv4(
    url: &str,
    timeout: Duration,
    creds: &SigV4Credentials<'_>,
    custom_headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.get(url).build()?;
    crate::sigv4::sign_request(
        &mut request,
        b"",
        creds.access_key,
        creds.secret_key,
        creds.session_token,
        creds.region,
        creds.service,
    );
    apply_unsigned_headers(&mut request, custom_headers);
    text_reply(timeout, client.execute(request)).await
}

pub async fn get_text(
    url: &str,
    timeout: Duration,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    text_reply(timeout, request.send()).await
}

/// DELETE with no body, as the Gemini Omni stored-job delete requires
/// (HANDOFF-109). Transport errors go through `From<reqwest::Error>`, which
/// strips the URL, so a `?key=` URL never prints (VULN-001).
pub(crate) async fn delete_text(
    url: &str,
    timeout: Duration,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.delete(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    text_reply(timeout, request.send()).await
}

pub async fn get_bytes(
    url: &str,
    timeout: Duration,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, Vec<u8>), Error> {
    let client = client_for(timeout)?;
    let mut request = client.get(url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    bytes_reply(timeout, request.send()).await
}

pub async fn post_multipart(
    url: &str,
    timeout: Duration,
    form: reqwest::multipart::Form,
    headers: &[(String, String)],
) -> Result<(reqwest::StatusCode, String), Error> {
    let client = client_for(timeout)?;
    let mut request = client.post(url).multipart(form);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    text_reply(timeout, request.send()).await
}
