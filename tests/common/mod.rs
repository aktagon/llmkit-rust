// Shared mock-server helpers for the typed-builder smoke tests
// (`tests/prompt.rs`) and the request wire-conformance drivers
// (`tests/request_wire.rs`). A `tests/common/` directory module (not
// `tests/common.rs`) so cargo does not compile it as its own test binary.

// Generated canonical wire-fixture inputs (ontology/wire-fixtures.ttl,
// plan 039). dead_code allowed: only the request_wire binary uses them.
#[allow(dead_code)]
pub mod wire_inputs;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use serde_json::Value;

pub struct TestResponse {
    pub status_line: &'static str,
    pub body: String,
    pub headers: Vec<(&'static str, &'static str)>,
}

pub struct TestExchange {
    pub assert_request: Box<dyn Fn(String, Value) + Send + 'static>,
    pub response: TestResponse,
}

pub fn serve_once<F>(assert_request: F, response: TestResponse) -> String
where
    F: Fn(String, Value) + Send + 'static,
{
    serve_sequence(vec![TestExchange {
        assert_request: Box::new(assert_request),
        response,
    }])
}

pub fn serve_sequence(exchanges: Vec<TestExchange>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    thread::spawn(move || serve_on(listener, exchanges));
    format!("http://{}", addr)
}

/// Like `serve_sequence` but the exchange list is built from the bound base
/// URL — needed when a served response body must embed the mock server's own
/// address (Veo's done op carries a Files-API download URI back into the
/// mock). The builder closure receives `http://127.0.0.1:<port>`.
#[allow(dead_code)]
pub fn serve_sequence_with_url<F>(build: F) -> String
where
    F: FnOnce(&str) -> Vec<TestExchange>,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let base = format!("http://{addr}");
    let exchanges = build(&base);
    thread::spawn(move || serve_on(listener, exchanges));
    base
}

fn serve_on(listener: TcpListener, exchanges: Vec<TestExchange>) {
    for exchange in exchanges {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_http_request(&mut stream);
        let split = request
            .find("\r\n\r\n")
            .expect("http request separator present");
        let body_text = request[split + 4..].to_string();
        let json: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);
        (exchange.assert_request)(request, json);

        let mut response_text = format!(
            "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            exchange.response.status_line,
            exchange.response.body.len()
        );
        for (name, value) in exchange.response.headers {
            response_text.push_str(&format!("{name}: {value}\r\n"));
        }
        response_text.push_str("\r\n");
        response_text.push_str(&exchange.response.body);
        stream
            .write_all(response_text.as_bytes())
            .expect("write response");
    }
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];

    loop {
        let bytes_read = stream.read(&mut chunk).expect("read");
        if bytes_read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);
        if let Some(split) = find_header_end(&buffer) {
            let header_text = String::from_utf8_lossy(&buffer[..split]).to_string();
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    let lower = line.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let body_len = buffer.len().saturating_sub(split + 4);
            if body_len >= content_length {
                break;
            }
        }
    }

    String::from_utf8_lossy(&buffer).to_string()
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}
