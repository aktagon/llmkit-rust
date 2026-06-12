// Typed-builder smoke tests for `c.video().<chain>.submit(...)` + the
// VideoHandle::wait extension trait (ADR-034). Mirror of go/video_test.go.
//
// Grok (VideoGrok) is reachable via a base_url override and tested
// end-to-end: a POST submit followed by polled GETs (pending → done). The
// shared serve_sequence helper serves each request on its own connection,
// so the poll loop talks to the same mock across the sequence.

mod common;

use std::time::Duration;

use common::{serve_sequence, serve_sequence_with_url, TestExchange, TestResponse};
use llmkit::builders::{anthropic, google, grok, minimax, qwen, together, zhipu};
use llmkit::builders::VideoHandleExt;
use llmkit::{submit_video, wait_video, Part, Provider, ProviderName, VideoPoll, VideoRequest};

const GROK_VIDEO_MODEL: &str = "grok-imagine-video";
const ZHIPU_VIDEO_MODEL: &str = "cogvideox-3";
const TOGETHER_VIDEO_MODEL: &str = "minimax/video-01-director";
const QWEN_VIDEO_MODEL: &str = "wan2.2-t2v-plus";

// Fast poll cadence so pending → done resolves immediately in tests.
fn fast_poll() -> VideoPoll {
    VideoPoll {
        interval: Duration::from_millis(1),
        timeout: Duration::from_secs(5),
    }
}

fn json_response(body: serde_json::Value) -> TestResponse {
    TestResponse {
        status_line: "HTTP/1.1 200 OK",
        body: body.to_string(),
        headers: Vec::new(),
    }
}

// Submit exchange: assert POST {model, prompt} carries the bearer token and
// the model, then return {request_id}. doneBody is served after pendingPolls
// pending GET responses.
fn grok_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /v1/videos/generations"),
                "submit must POST gen endpoint: {request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token"),
                "submit must carry bearer auth: {request}"
            );
            assert_eq!(body["model"], GROK_VIDEO_MODEL);
            assert!(
                body["prompt"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                "submit body must carry a non-empty prompt: {body}"
            );
        }),
        response: json_response(serde_json::json!({ "request_id": "vid-123" })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /v1/videos/vid-123"),
                    "poll must GET the per-id endpoint: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "status": "pending" })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /v1/videos/vid-123"),
                "poll must GET the per-id endpoint: {request}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#[tokio::test]
async fn submit_and_wait_grok_resolves_url_delivery() {
    let done = serde_json::json!({
        "status": "done",
        "video": { "url": "https://vidgen.x.ai/abc/video.mp4", "duration": 8 },
        "model": GROK_VIDEO_MODEL,
    });
    let url = serve_sequence(grok_exchanges(2, done));

    let mut client = grok("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(GROK_VIDEO_MODEL)
        .submit("a drone shot over the alps, 8s")
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "vid-123");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    assert_eq!(resp.videos[0].url, "https://vidgen.x.ai/abc/video.mp4");
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
    assert_eq!(resp.videos[0].duration_seconds, 8);
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

// Zhipu CogVideoX: submit returns the poll handle as the top-level `id`
// (its own `request_id` is present but is NOT the poll key); the poll GETs
// /v4/async-result/{id} until task_status == SUCCESS with the finished video
// at video_result[0].url (url delivery).
fn zhipu_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /v4/videos/generations"),
                "submit must POST the CogVideoX gen endpoint: {request}"
            );
            assert_eq!(body["model"], ZHIPU_VIDEO_MODEL);
            assert!(
                body["prompt"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                "submit body must carry a non-empty prompt: {body}"
            );
        }),
        response: json_response(serde_json::json!({
            "id": "zhipu-vid-1",
            "request_id": "rq-xyz",
            "task_status": "PROCESSING"
        })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /v4/async-result/zhipu-vid-1"),
                    "poll must GET the async-result endpoint: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "task_status": "PROCESSING" })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /v4/async-result/zhipu-vid-1"),
                "poll must GET the async-result endpoint: {request}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#[tokio::test]
async fn submit_and_wait_zhipu_resolves_url_delivery() {
    let done = serde_json::json!({
        "task_status": "SUCCESS",
        "video_result": [
            { "url": "https://cogvideo.bigmodel.cn/abc/v.mp4", "cover_image_url": "https://cogvideo.bigmodel.cn/abc/c.jpg" }
        ],
        "model": ZHIPU_VIDEO_MODEL,
    });
    let url = serve_sequence(zhipu_exchanges(2, done));

    let mut client = zhipu("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(ZHIPU_VIDEO_MODEL)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "zhipu-vid-1");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    assert_eq!(resp.videos[0].url, "https://cogvideo.bigmodel.cn/abc/v.mp4");
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

#[tokio::test]
async fn wait_zhipu_fail_status_errors() {
    let url = serve_sequence(zhipu_exchanges(0, serde_json::json!({ "task_status": "FAIL" })));

    let mut client = zhipu("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(ZHIPU_VIDEO_MODEL)
        .submit("blocked prompt")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll()).await.expect_err("FAIL must error");
    assert!(
        err.to_string().contains("video generation failed"),
        "unexpected error: {err}"
    );
}

// Together: submit returns the poll handle as the top-level `id` with
// status=queued; the poll GETs /v2/videos/{id} until status == completed with
// the finished video at outputs.video_url (url delivery).
fn together_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /v2/videos"),
                "submit must POST the Together video endpoint: {request}"
            );
            assert_eq!(body["model"], TOGETHER_VIDEO_MODEL);
            assert!(
                body["prompt"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                "submit body must carry a non-empty prompt: {body}"
            );
        }),
        response: json_response(serde_json::json!({
            "id": "together-vid-1",
            "status": "queued"
        })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /v2/videos/together-vid-1"),
                    "poll must GET the video endpoint: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "status": "in_progress" })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /v2/videos/together-vid-1"),
                "poll must GET the video endpoint: {request}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#[tokio::test]
async fn submit_and_wait_together_resolves_url_delivery() {
    let done = serde_json::json!({
        "status": "completed",
        "outputs": { "video_url": "https://api.together.xyz/files/v.mp4" },
        "model": TOGETHER_VIDEO_MODEL,
    });
    let url = serve_sequence(together_exchanges(2, done));

    let mut client = together("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(TOGETHER_VIDEO_MODEL)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "together-vid-1");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    assert_eq!(resp.videos[0].url, "https://api.together.xyz/files/v.mp4");
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

#[tokio::test]
async fn wait_together_cancelled_status_errors() {
    let url = serve_sequence(together_exchanges(0, serde_json::json!({ "status": "cancelled" })));

    let mut client = together("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(TOGETHER_VIDEO_MODEL)
        .submit("blocked prompt")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll()).await.expect_err("cancelled must error");
    assert!(
        err.to_string().contains("video generation cancelled"),
        "unexpected error: {err}"
    );
}

// Qwen (DashScope): submit POSTs the NESTED {model, input:{prompt}} body with
// the required X-DashScope-Async: enable header; the poll handle is at
// output.task_id (dotted path). Poll GETs /api/v1/tasks/{id} until
// output.task_status == SUCCEEDED with the finished video at output.video_url
// (url delivery).
fn qwen_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /api/v1/services/aigc/video-generation/video-synthesis"),
                "submit must POST the DashScope video endpoint: {request}"
            );
            // Load-bearing async header asserted in-driver (mirrors Anthropic
            // beta-header). The raw request string carries the headers.
            assert!(
                request.to_lowercase().contains("x-dashscope-async: enable"),
                "submit must carry X-DashScope-Async: enable: {request}"
            );
            assert_eq!(body["model"], QWEN_VIDEO_MODEL);
            assert_eq!(
                body["input"]["prompt"].as_str(),
                Some("a drone shot over the alps"),
                "submit body must nest the prompt under input: {body}"
            );
            assert!(
                body.get("prompt").is_none(),
                "submit body must NOT carry a top-level prompt (nested under input): {body}"
            );
        }),
        response: json_response(serde_json::json!({
            "output": { "task_id": "qwen-vid-1", "task_status": "PENDING" },
            "request_id": "req-1"
        })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /api/v1/tasks/qwen-vid-1"),
                    "poll must GET the task endpoint: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "output": { "task_status": "RUNNING" } })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /api/v1/tasks/qwen-vid-1"),
                "poll must GET the task endpoint: {request}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#[tokio::test]
async fn submit_and_wait_qwen_resolves_url_delivery() {
    let done = serde_json::json!({
        "output": {
            "task_status": "SUCCEEDED",
            "video_url": "https://dashscope-result.oss-cn.aliyuncs.com/v.mp4"
        }
    });
    let url = serve_sequence(qwen_exchanges(2, done));

    let mut client = qwen("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(QWEN_VIDEO_MODEL)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "qwen-vid-1");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    assert_eq!(
        resp.videos[0].url,
        "https://dashscope-result.oss-cn.aliyuncs.com/v.mp4"
    );
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

#[tokio::test]
async fn wait_qwen_failed_status_errors() {
    let url = serve_sequence(qwen_exchanges(
        0,
        serde_json::json!({ "output": { "task_status": "FAILED" } }),
    ));

    let mut client = qwen("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(QWEN_VIDEO_MODEL)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll()).await.expect_err("failed must error");
    assert!(
        err.to_string().contains("video generation FAILED"),
        "unexpected error: {err}"
    );
}

const MINIMAX_VIDEO_MODEL: &str = "MiniMax-Hailuo-2.3";

// minimax_exchanges serves the MiniMax two-hop flow: submit -> {task_id};
// query poll returns Processing for pending_polls calls, then {status:Success,
// file_id} (file_id as a JSON number); the file-retrieve hop returns the
// download URL. When fail is set the (single) poll returns {status:Fail}.
fn minimax_exchanges(pending_polls: usize, download_url: &'static str, fail: bool) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /v1/video_generation"),
                "submit must POST the MiniMax video endpoint: {request}"
            );
            assert_eq!(body["model"], MINIMAX_VIDEO_MODEL);
            assert!(
                body["prompt"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                "submit body must carry a top-level prompt: {body}"
            );
        }),
        response: json_response(serde_json::json!({
            "task_id": "mmtask-1", "base_resp": { "status_code": 0 }
        })),
    });

    if fail {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /v1/query/video_generation?task_id=mmtask-1"),
                    "poll must GET the query endpoint: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "status": "Fail" })),
        });
        return exchanges;
    }

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /v1/query/video_generation?task_id=mmtask-1"),
                    "poll must GET the query endpoint: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "status": "Processing" })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /v1/query/video_generation?task_id=mmtask-1"),
                "poll must GET the query endpoint: {request}"
            );
        }),
        response: json_response(serde_json::json!({ "status": "Success", "file_id": 99887766 })),
    });

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /v1/files/retrieve?file_id=99887766"),
                "file hop must GET the file-retrieve endpoint with the file_id: {request}"
            );
        }),
        response: json_response(serde_json::json!({ "file": { "download_url": download_url } })),
    });

    exchanges
}

#[tokio::test]
async fn submit_and_wait_minimax_two_hop_resolves_url() {
    let url = serve_sequence(minimax_exchanges(2, "https://files.minimax.io/abc/v.mp4", false));

    let mut client = minimax("test-token");
    client.provider.base_url = Some(url); // override wins (Option D)

    let handle = client
        .video()
        .model(MINIMAX_VIDEO_MODEL)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "mmtask-1");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    // The URL came from the second (file-retrieve) hop, not the poll body.
    assert_eq!(resp.videos[0].url, "https://files.minimax.io/abc/v.mp4");
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

#[tokio::test]
async fn wait_minimax_fail_status_errors() {
    let url = serve_sequence(minimax_exchanges(0, "", true));

    let mut client = minimax("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(MINIMAX_VIDEO_MODEL)
        .submit("blocked prompt")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll()).await.expect_err("Fail must error");
    assert!(
        err.to_string().contains("video generation failed"),
        "unexpected error: {err}"
    );
}

const VEO_VIDEO_MODEL: &str = "veo-3.1-generate-preview";

// Raw mp4 payload served by the Veo download hop (valid UTF-8 so it round-trips
// through the String-bodied mock harness).
const VEO_VIDEO_BYTES: &str = "\u{0}\u{0}\u{0}\u{18}ftypmp42 fake mp4 payload";

// veo_exchanges serves the Google Veo LRO flow against the mock at `base`:
// submit -> {name:"models/.../operations/op-1"}; operation poll returns
// {done:false} for pending_polls calls, then a done op whose response carries
// a Files-API video.uri pointing back at the mock (download delivery); the
// download hop returns raw mp4 bytes. Every request must carry the ?key=
// query-param auth (Google is the first video provider that is NOT bearer-
// header). The download uri carries a pre-existing ?alt=media so the test also
// witnesses the ?->& auth-append branch.
fn veo_exchanges(base: &str, pending_polls: usize) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains(
                    "POST /v1beta/models/veo-3.1-generate-preview:predictLongRunning?key=test-token"
                ),
                "submit must POST the Veo predictLongRunning endpoint with ?key=: {request}"
            );
            assert!(
                body.get("model").is_none(),
                "Veo submit body must NOT carry a model field: {body}"
            );
            let instances = body["instances"].as_array().expect("instances array");
            assert_eq!(instances.len(), 1, "expected one instance: {body}");
            assert!(
                instances[0]["prompt"]
                    .as_str()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false),
                "instances[0].prompt must be non-empty: {body}"
            );
        }),
        response: json_response(serde_json::json!({
            "name": "models/veo-3.1-generate-preview/operations/op-1"
        })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /v1beta/models/veo-3.1-generate-preview/operations/op-1?key=test-token"),
                    "poll must GET the operation endpoint with ?key=: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "done": false })),
        });
    }

    let download_uri = format!("{base}/v1beta/files/vid-file:download?alt=media");
    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /v1beta/models/veo-3.1-generate-preview/operations/op-1?key=test-token"),
                "poll must GET the operation endpoint with ?key=: {request}"
            );
        }),
        response: json_response(serde_json::json!({
            "done": true,
            "response": {
                "generateVideoResponse": {
                    "generatedSamples": [
                        { "video": { "uri": download_uri } }
                    ]
                }
            }
        })),
    });

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            // The pre-existing ?alt=media must survive the auth append (?->&).
            assert!(
                request.contains("GET /v1beta/files/vid-file:download?alt=media&key=test-token"),
                "download hop must GET the file uri with alt=media preserved and &key= appended: {request}"
            );
        }),
        response: TestResponse {
            status_line: "HTTP/1.1 200 OK",
            body: VEO_VIDEO_BYTES.to_string(),
            headers: Vec::new(),
        },
    });

    exchanges
}

#[tokio::test]
async fn submit_and_wait_veo_download_delivery() {
    let url = serve_sequence_with_url(|base| veo_exchanges(base, 2));

    let mut client = google("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(VEO_VIDEO_MODEL)
        .submit("a drone shot over the alps at sunrise")
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "models/veo-3.1-generate-preview/operations/op-1");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    // Download delivery filled bytes and cleared url (source-XOR, VID-004).
    assert_eq!(resp.videos[0].bytes, VEO_VIDEO_BYTES.as_bytes());
    assert!(
        resp.videos[0].url.is_empty(),
        "download delivery must clear url after fetching bytes"
    );
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
}

#[tokio::test]
async fn wait_veo_failed_operation_errors_with_message() {
    let exchanges = vec![
        TestExchange {
            assert_request: Box::new(|_request: String, _body| {}),
            response: json_response(serde_json::json!({
                "name": "models/veo-3.1-generate-preview/operations/op-1"
            })),
        },
        TestExchange {
            assert_request: Box::new(|_request: String, _body| {}),
            response: json_response(serde_json::json!({
                "done": true,
                "error": { "code": 3, "message": "prompt blocked by safety filter" }
            })),
        },
    ];
    let url = serve_sequence(exchanges);

    let mut client = google("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(VEO_VIDEO_MODEL)
        .submit("blocked prompt")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll())
        .await
        .expect_err("done+error operation must error");
    assert!(
        err.to_string().contains("prompt blocked by safety filter"),
        "error must surface the operation message, got: {err}"
    );
}

#[tokio::test]
async fn wait_via_extension_trait_resolves() {
    // Exercises the VideoHandleExt::wait method-style call site (default
    // 5s interval) with zero pending polls so it returns promptly.
    let done = serde_json::json!({
        "status": "done",
        "video": { "url": "https://vidgen.x.ai/t.mp4" },
    });
    let url = serve_sequence(grok_exchanges(0, done));

    let mut client = grok("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(GROK_VIDEO_MODEL)
        .text("a calm lake at dawn")
        .submit("")
        .await
        .expect("submit succeeds");

    let resp = handle.wait().await.expect("wait succeeds");
    assert_eq!(resp.videos[0].url, "https://vidgen.x.ai/t.mp4");
}

#[tokio::test]
async fn raw_opt_in_captures_poll_body() {
    let done = serde_json::json!({
        "status": "done",
        "video": { "url": "https://vidgen.x.ai/x.mp4" },
    });
    let url = serve_sequence(grok_exchanges(0, done));

    let mut client = grok("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(GROK_VIDEO_MODEL)
        .raw()
        .submit("a sunrise timelapse")
        .await
        .expect("submit succeeds");
    assert!(handle.raw, "chain .raw() must propagate onto the handle");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    let raw = resp.raw.expect("raw poll body captured");
    assert!(raw.get("video").is_some());
}

#[tokio::test]
async fn wait_failed_job_errors_with_message() {
    let done = serde_json::json!({
        "status": "failed",
        "error": { "code": "invalid_argument", "message": "prompt blocked by moderation" },
    });
    let url = serve_sequence(grok_exchanges(0, done));

    let mut client = grok("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(GROK_VIDEO_MODEL)
        .submit("blocked prompt")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll())
        .await
        .expect_err("failed job must error");
    assert!(
        err.to_string().contains("prompt blocked by moderation"),
        "error must surface the provider message, got: {err}"
    );
}

#[tokio::test]
async fn submit_requires_model() {
    let result = grok("test-token").video().submit("no model set").await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "model"),
        other => panic!("expected model validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn submit_rejects_unknown_model() {
    let result = grok("test-token")
        .video()
        .model("grok-imagine-nope")
        .submit("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "model"),
        other => panic!("expected model validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn submit_rejects_unsupported_provider() {
    let result = anthropic("test-token")
        .video()
        .model(GROK_VIDEO_MODEL)
        .submit("x")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "provider"),
        other => panic!("expected provider validation error, got {:?}", other),
    }
}

fn provider() -> Provider {
    Provider {
        name: ProviderName::Grok,
        api_key: "test-token".into(),
        model: None,
        base_url: Some("http://unused".into()),
    }
}

// The Video builder exposes no .lyrics() chain method (by design), so the
// lyrics rejection drives the crate-public free function directly with a
// hand-built request.
#[tokio::test]
async fn submit_rejects_lyrics_part() {
    let req = VideoRequest {
        model: GROK_VIDEO_MODEL.into(),
        prompt: String::new(),
        parts: vec![Part::lyrics("la la la")],
    };
    let result = submit_video(&provider(), &req, &[], false).await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "parts"),
        other => panic!("expected parts validation error, got {:?}", other),
    }
}

#[tokio::test]
async fn submit_enforces_prompt_parts_xor() {
    // neither
    let neither = VideoRequest {
        model: GROK_VIDEO_MODEL.into(),
        ..VideoRequest::default()
    };
    assert!(
        submit_video(&provider(), &neither, &[], false).await.is_err(),
        "neither prompt nor parts must error"
    );

    // both
    let both = VideoRequest {
        model: GROK_VIDEO_MODEL.into(),
        prompt: "x".into(),
        parts: vec![Part::text("y")],
    };
    assert!(
        submit_video(&provider(), &both, &[], false).await.is_err(),
        "both prompt and parts must error"
    );
}
