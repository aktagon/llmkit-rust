//!
//!
//!
//!
//!

//
//
//

use llmkit::builders::{
    anthropic, google, grok, openai, Agent, Image, ImageData, MediaRef, Text, Upload,
};

//
//
//
//

//

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

///
///
fn mock_server(response_body: String) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buffer = vec![0u8; 8192];
        let mut total = Vec::new();
        loop {
            let n = stream.read(&mut buffer).unwrap_or(0);
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buffer[..n]);
            //
            //
            if let Some(headers_end) = total
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
            {
                let header = String::from_utf8_lossy(&total[..headers_end]);
                let cl = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if total.len() >= headers_end + 4 + cl {
                    break;
                }
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    format!("http://{}", addr)
}

#
fn phase3_text_prompt_wires_against_legacy() {
    let body = r#"{"content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
    rt().block_on(async {
        let url = mock_server(body.into());
        let mut client = anthropic("k");
        client.provider.base_url = Some(url);
        let resp = client
            .text()
            .system("be terse")
            .max_tokens(50)
            .prompt("hello")
            .await
            .expect("prompt ok");
        assert_eq!(resp.text, "ok");
    });
}

#
fn phase3_text_batch_returns_handle() {
    let body = r#"{"id":"msgbatch_123"}"#;
    rt().block_on(async {
        let url = mock_server(body.into());
        let mut client = anthropic("k");
        client.provider.base_url = Some(url);
        let handle = client
            .text()
            .system("s")
            .batch(vec!["p1".to_string(), "p2".to_string()])
            .await
            .expect("submit ok");
        assert_eq!(handle.id, "msgbatch_123");
        assert_eq!(handle.provider.api_key, "k");
    });
}

#
fn phase3_upload_run_validation() {
    rt().block_on(async {
        let c = openai("k");

        //
        let err = c.upload().run().await.expect_err("empty should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("exactly one of"), "got: {}", msg);

        //
        let err = openai("k")
            .upload()
            .bytes(vec![1])
            .path("/p")
            .run()
            .await
            .expect_err("both should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("mutually exclusive"), "got: {}", msg);

        //
        let err = openai("k")
            .upload()
            .bytes(vec![1])
            .run()
            .await
            .expect_err("bytes without filename should error");
        let msg = format!("{:?}", err);
        assert!(msg.contains("filename"), "got: {}", msg);
    });
}

///
///
fn mock_server_capturing(
    response_body: String,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    let captured = Arc::new(Mutex::new(Vec::<u8>::new()));
    let captured_writer = Arc::clone(&captured);

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buffer = vec![0u8; 8192];
        let mut total = Vec::new();
        loop {
            let n = stream.read(&mut buffer).unwrap_or(0);
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buffer[..n]);
            if let Some(headers_end) = total.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&total[..headers_end]);
                let cl = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if total.len() >= headers_end + 4 + cl {
                    break;
                }
            }
        }
        *captured_writer.lock().unwrap() = total;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(response.as_bytes());
    });
    (format!("http://{}", addr), captured)
}

#
fn phase3_upload_run_bytes_round_trips() {
    let (url, captured) = mock_server_capturing(r#"{"id":"file-zzz"}"#.into());
    rt().block_on(async {
        let mut c = openai("k");
        c.provider.base_url = Some(url);
        let result = c
            .upload()
            .bytes(b"hello".to_vec())
            .filename("note.txt")
            .mime_type("text/plain")
            .run()
            .await
            .expect("bytes upload should succeed");
        assert_eq!(result.id, "file-zzz");
    });

    let body = captured.lock().unwrap().clone();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("filename=\"note.txt\""), "body: {}", body_str);
    assert!(body_str.contains("text/plain"), "body: {}", body_str);
    assert!(body_str.contains("hello"), "body: {}", body_str);
}

#
fn phase3_text_stream_wires_via_callback() {
    //
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"He\"}}]}\n\n\
                data: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\n\
                data: [DONE]\n\n"
        .to_string();
    //
    //
    //
    rt().block_on(async {
        let url = mock_server(body);
        let mut client = openai("k");
        client.provider.base_url = Some(url);

        let mut chunks: Vec<String> = Vec::new();
        let chunks_ref = &mut chunks;
        let _ = client
            .text()
            .stream("hi", |chunk: &str| {
                chunks_ref.push(chunk.to_string());
            })
            .await;
        assert!(
            !chunks.is_empty(),
            "stream callback should receive at least one chunk"
        );
        assert_eq!(chunks[0], "He");
    });
}

//
//
//
#
fn stream_usage_opt_in_openai() {
    let (url, captured) = mock_server_capturing("data: [DONE]\n\n".into());
    rt().block_on(async {
        let mut client = openai("k");
        client.provider.base_url = Some(url);
        let _ = client.text().model("m").stream("hi", |_c: &str| {}).await;
    });
    let body = captured.lock().unwrap().clone();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        body_str.contains("\"stream_options\":{\"include_usage\":true}"),
        "expected stream_options in body: {}",
        body_str
    );
}

#
fn stream_usage_opt_in_grok_omitted() {
    let (url, captured) = mock_server_capturing("data: [DONE]\n\n".into());
    rt().block_on(async {
        let mut client = grok("k");
        client.provider.base_url = Some(url);
        let _ = client.text().model("m").stream("hi", |_c: &str| {}).await;
    });
    let body = captured.lock().unwrap().clone();
    let body_str = String::from_utf8_lossy(&body);
    assert!(
        !body_str.contains("stream_options"),
        "expected no stream_options for Grok: {}",
        body_str
    );
}

//
//
//

#
fn text_history_chain_returns_text_builder() {
    //
    //
    //
    //
    //
    let msgs = vec![
        llmkit::Message {
            role: "user".to_string(),
            content: "earlier turn".to_string(),
            ..Default::default()
        },
        llmkit::Message {
            role: "assistant".to_string(),
            content: "earlier reply".to_string(),
            ..Default::default()
        },
    ];
    let _t: Text = anthropic("k").text().history(msgs).system("be terse");
}

#
fn agent_reset_is_a_no_op_before_first_prompt() {
    //
    //
    //
    //
    let mut bot: Agent = anthropic("k").agent().system("be terse").max_tokens(50);
    bot.reset();
    //
    bot.reset();
}

//

#
fn type_aliases_constructible() {
    let _: ImageData = ImageData::default();
    let _: MediaRef = MediaRef::default();
    let _: Text = google("k").text();
    let _: Image = google("k").image();
    let _: Agent = google("k").agent();
    let _: Upload = google("k").upload();
}

//

const HDR_FLASH_MODEL: &str = "gemini-3.1-flash-image-preview";

///
///
///
#
fn add_header_reaches_wire_text_path() {
    let resp = r#"{"content":[{"type":"text","text":"pong"}],"usage":{"input_tokens":5,"output_tokens":1}}"#;
    let (url, captured) = mock_server_capturing(resp.into());
    rt().block_on(async {
        let c = anthropic("test-key")
            .base_url(url)
            .add_header("cf-aig-authorization", "Bearer gw-token");
        let r = c.text().prompt("ping").await.expect("prompt ok");
        assert_eq!(r.text, "pong");
    });
    let raw = captured.lock().unwrap().clone();
    let req = String::from_utf8_lossy(&raw).to_ascii_lowercase();
    assert!(req.contains("x-api-key: test-key"), "auth header missing: {req}");
    assert!(
        req.contains("cf-aig-authorization: bearer gw-token"),
        "custom header missing: {req}"
    );
}

///
///
#
fn add_header_reaches_wire_image_path() {
    let resp = r#"{"candidates":[{"content":{"parts":[{"inlineData":{"mimeType":"image/png","data":"aGVsbG8="}}]}}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":1290}}"#;
    let (url, captured) = mock_server_capturing(resp.into());
    rt().block_on(async {
        let c = google("test-key")
            .base_url(url)
            .add_header("cf-aig-authorization", "Bearer gw-token");
        let r = c
            .image()
            .model(HDR_FLASH_MODEL)
            .generate("A nano banana dish")
            .await
            .expect("generate ok");
        assert_eq!(r.images.len(), 1);
    });
    let raw = captured.lock().unwrap().clone();
    let req = String::from_utf8_lossy(&raw).to_ascii_lowercase();
    assert!(
        req.contains("cf-aig-authorization: bearer gw-token"),
        "custom header missing on image path: {req}"
    );
}

///
///
#
fn add_header_does_not_clobber_provider_auth() {
    let resp = r#"{"content":[{"type":"text","text":"pong"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
    let (url, captured) = mock_server_capturing(resp.into());
    rt().block_on(async {
        let c = anthropic("test-key")
            .base_url(url)
            .add_header("x-api-key", "attacker-override");
        let _ = c.text().prompt("ping").await.expect("prompt ok");
    });
    let raw = captured.lock().unwrap().clone();
    let req = String::from_utf8_lossy(&raw);
    assert!(
        req.contains("x-api-key: test-key"),
        "caller clobbered provider auth: {req}"
    );
    assert!(
        !req.contains("attacker-override"),
        "attacker override reached the wire: {req}"
    );
}

///
///
#
fn add_header_different_cased_collision_cannot_clobber_auth() {
    let resp = r#"{"content":[{"type":"text","text":"pong"}],"usage":{"input_tokens":1,"output_tokens":1}}"#;
    let (url, captured) = mock_server_capturing(resp.into());
    rt().block_on(async {
        let c = anthropic("test-key")
            .base_url(url)
            .add_header("X-API-KEY", "attacker-override");
        let _ = c.text().prompt("ping").await.expect("prompt ok");
    });
    let raw = captured.lock().unwrap().clone();
    let req = String::from_utf8_lossy(&raw);
    assert!(
        req.contains("x-api-key: test-key"),
        "different-cased caller header clobbered provider auth: {req}"
    );
    assert!(
        !req.contains("attacker-override"),
        "attacker override reached the wire: {req}"
    );
}
