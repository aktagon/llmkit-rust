//
//
//
//
//
//
//

mod common;

use std::time::Duration;

use common::{serve_sequence, serve_sequence_with_url, TestExchange, TestResponse};
use llmkit::builders::{
    anthropic, bedrock, google, grok, minimax, pixverse, qwen, together, vertex, vidu, zhipu,
};
use llmkit::builders::VideoHandleExt;
use llmkit::{submit_video, wait_video, Part, Provider, ProviderName, VideoPoll, VideoRequest};

const GROK_VIDEO_MODEL: &str = "grok-imagine-video";
const ZHIPU_VIDEO_MODEL: &str = "cogvideox-3";
const TOGETHER_VIDEO_MODEL: &str = "minimax/video-01-director";
const QWEN_VIDEO_MODEL: &str = "wan2.2-t2v-plus";
const VIDU_VIDEO_MODEL: &str = "viduq3-pro";
const PIXVERSE_VIDEO_MODEL: &str = "v4.5";
const PIXVERSE_VIDEO_ID: i64 = 318633193768896;

//
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

//
//
//
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

#
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

//
//
const GROK_SEED_PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGM4YWQEAALyAS2saifrAAAAAElFTkSuQmCC";

//
//
fn grok_i2v_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /v1/videos/generations"),
                "submit must POST gen endpoint: {request}"
            );
            assert_eq!(body["model"], GROK_VIDEO_MODEL);
            assert_eq!(
                body["image"]["url"],
                serde_json::json!(format!("data:image/png;base64,{GROK_SEED_PNG_B64}")),
                "seed frame must inline as a data URL at image.url: {body}"
            );
        }),
        response: json_response(serde_json::json!({ "request_id": "vid-123" })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(request.contains("GET /v1/videos/vid-123"), "poll: {request}");
            }),
            response: json_response(serde_json::json!({ "status": "pending" })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(request.contains("GET /v1/videos/vid-123"), "poll: {request}");
        }),
        response: json_response(done_body),
    });

    exchanges
}

#
async fn submit_and_wait_grok_image_to_video() {
    use base64::Engine;
    let seed = base64::engine::general_purpose::STANDARD
        .decode(GROK_SEED_PNG_B64)
        .expect("decode seed PNG");
    let done = serde_json::json!({
        "status": "done",
        "video": { "url": "https://vidgen.x.ai/i2v/out.mp4", "duration": 6 },
    });
    let url = serve_sequence(grok_i2v_exchanges(1, done));

    let mut client = grok("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(GROK_VIDEO_MODEL)
        .image("image/png", seed)
        .submit("animate the still: slow push-in")
        .await
        .expect("i2v submit succeeds");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos[0].url, "https://vidgen.x.ai/i2v/out.mp4");
}

#
async fn video_image_part_on_text_only_model_rejects() {
    use base64::Engine;
    let seed = base64::engine::general_purpose::STANDARD
        .decode(GROK_SEED_PNG_B64)
        .expect("decode seed PNG");
    //
    let err = zhipu("test-token")
        .video()
        .model(ZHIPU_VIDEO_MODEL)
        .image("image/png", seed)
        .submit("animate this")
        .await
        .expect_err("text-to-video-only model must reject an image part");
    assert!(
        err.to_string().contains("text-to-video-only"),
        "expected text-to-video-only rejection, got: {err}"
    );
}

#
async fn video_rejects_multiple_seed_frames() {
    use base64::Engine;
    let seed = base64::engine::general_purpose::STANDARD
        .decode(GROK_SEED_PNG_B64)
        .expect("decode seed PNG");
    let err = grok("test-token")
        .video()
        .model(GROK_VIDEO_MODEL)
        .image("image/png", seed.clone())
        .image("image/png", seed)
        .submit("animate this")
        .await
        .expect_err("a second seed frame must be rejected");
    assert!(
        err.to_string().contains("single seed frame"),
        "expected single seed frame rejection, got: {err}"
    );
}

//
//
//
//
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

#
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

#
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

//
//
//
//
fn vidu_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /ent/v2/text2video"),
                "submit must POST the Vidu text2video endpoint: {request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: token test-token"),
                "submit must carry Token auth (not Bearer): {request}"
            );
            assert_eq!(body["model"], VIDU_VIDEO_MODEL);
            assert!(
                body["prompt"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                "submit body must carry a non-empty prompt: {body}"
            );
        }),
        response: json_response(serde_json::json!({
            "task_id": "vidu-task-1",
            "state": "created"
        })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains("GET /ent/v2/tasks/vidu-task-1/creations"),
                    "poll must GET the task-creations endpoint: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "state": "processing" })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains("GET /ent/v2/tasks/vidu-task-1/creations"),
                "poll must GET the task-creations endpoint: {request}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#
async fn submit_and_wait_vidu_resolves_url_delivery() {
    let done = serde_json::json!({
        "state": "success",
        "creations": [ { "url": "https://api.vidu.com/creations/abc/v.mp4" } ],
    });
    let url = serve_sequence(vidu_exchanges(2, done));

    let mut client = vidu("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(VIDU_VIDEO_MODEL)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");
    assert_eq!(handle.id, "vidu-task-1");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    assert_eq!(resp.videos[0].url, "https://api.vidu.com/creations/abc/v.mp4");
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

#
async fn wait_vidu_failed_state_errors() {
    let url = serve_sequence(vidu_exchanges(
        0,
        serde_json::json!({ "state": "failed", "err_code": "content_moderation" }),
    ));

    let mut client = vidu("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(VIDU_VIDEO_MODEL)
        .submit("blocked prompt")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll())
        .await
        .expect_err("failed state must error");
    assert!(
        err.to_string().contains("content_moderation"),
        "unexpected error: {err}"
    );
}

//
//
//
//
//
fn pixverse_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /openapi/v2/video/text/generate"),
                "submit must POST the PixVerse text-to-video endpoint: {request}"
            );
            let lower = request.to_ascii_lowercase();
            assert!(
                lower.contains("api-key: test-token"),
                "submit must carry the API-KEY header: {request}"
            );
            let trace = lower
                .split("ai-trace-id:")
                .nth(1)
                .map(|s| s.lines().next().unwrap_or("").trim())
                .unwrap_or("");
            assert!(
                !trace.is_empty(),
                "submit must carry a non-empty Ai-trace-id header: {request}"
            );
            assert_eq!(body["model"], PIXVERSE_VIDEO_MODEL);
            assert!(
                body["prompt"].as_str().map(|s| !s.is_empty()).unwrap_or(false),
                "submit body must carry a non-empty prompt: {body}"
            );
            //
            assert_eq!(body["duration"], 5);
            assert_eq!(body["quality"], "540p");
            assert_eq!(body["aspect_ratio"], "16:9");
        }),
        response: json_response(serde_json::json!({
            "ErrCode": 0,
            "ErrMsg": "success",
            "Resp": { "video_id": PIXVERSE_VIDEO_ID }
        })),
    });

    let poll_path = format!("GET /openapi/v2/video/result/{PIXVERSE_VIDEO_ID}");
    for _ in 0..pending_polls {
        let expected = poll_path.clone();
        exchanges.push(TestExchange {
            assert_request: Box::new(move |request: String, _body| {
                assert!(
                    request.contains(&expected),
                    "poll must GET the result endpoint: {request}"
                );
                assert!(
                    request.to_ascii_lowercase().contains("api-key: test-token"),
                    "poll must carry the API-KEY header: {request}"
                );
                assert!(
                    request.to_ascii_lowercase().contains("ai-trace-id:"),
                    "poll must carry an Ai-trace-id header: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "ErrCode": 0, "Resp": { "status": 5 } })),
        });
    }

    let expected = poll_path.clone();
    exchanges.push(TestExchange {
        assert_request: Box::new(move |request: String, _body| {
            assert!(
                request.contains(&expected),
                "poll must GET the result endpoint: {request}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#
async fn submit_and_wait_pixverse_resolves_url_delivery() {
    let done = serde_json::json!({
        "ErrCode": 0,
        "ErrMsg": "success",
        "Resp": { "id": PIXVERSE_VIDEO_ID, "status": 1, "url": "https://media.pixverse.ai/abc/v.mp4" },
    });
    let url = serve_sequence(pixverse_exchanges(2, done));

    let mut client = pixverse("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(PIXVERSE_VIDEO_MODEL)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");
    //
    assert_eq!(handle.id, "318633193768896");

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    assert_eq!(resp.videos[0].url, "https://media.pixverse.ai/abc/v.mp4");
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

#
async fn wait_pixverse_failed_status_errors() {
    let url = serve_sequence(pixverse_exchanges(
        0,
        serde_json::json!({ "ErrCode": 0, "Resp": { "status": 8 } }),
    ));

    let mut client = pixverse("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(PIXVERSE_VIDEO_MODEL)
        .submit("blocked prompt")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll())
        .await
        .expect_err("failed status must error");
    assert!(
        err.to_string().contains("status 8"),
        "unexpected error: {err}"
    );
}

#
async fn video_image_part_on_text_only_pixverse_rejects() {
    use base64::Engine;
    let seed = base64::engine::general_purpose::STANDARD
        .decode(GROK_SEED_PNG_B64)
        .expect("decode seed PNG");
    //
    let err = pixverse("test-token")
        .video()
        .model(PIXVERSE_VIDEO_MODEL)
        .image("image/png", seed)
        .submit("animate this")
        .await
        .expect_err("text-to-video-only model must reject an image part");
    assert!(
        err.to_string().contains("text-to-video-only"),
        "expected text-to-video-only rejection, got: {err}"
    );
}

#
async fn video_image_part_on_text_only_vidu_rejects() {
    use base64::Engine;
    let seed = base64::engine::general_purpose::STANDARD
        .decode(GROK_SEED_PNG_B64)
        .expect("decode seed PNG");
    //
    let err = vidu("test-token")
        .video()
        .model(VIDU_VIDEO_MODEL)
        .image("image/png", seed)
        .submit("animate this")
        .await
        .expect_err("text-to-video-only model must reject an image part");
    assert!(
        err.to_string().contains("text-to-video-only"),
        "expected text-to-video-only rejection, got: {err}"
    );
}

//
//
//
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

#
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

#
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

//
//
//
//
//
fn qwen_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /api/v1/services/aigc/video-generation/video-synthesis"),
                "submit must POST the DashScope video endpoint: {request}"
            );
            //
            //
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

#
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

#
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

//
//
//
//
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

#
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
    //
    assert_eq!(resp.videos[0].url, "https://files.minimax.io/abc/v.mp4");
    assert!(
        resp.videos[0].bytes.is_empty(),
        "url delivery must not download bytes"
    );
}

#
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

//
//
const VEO_VIDEO_BYTES: &str = "\u{0}\u{0}\u{0}\u{18}ftypmp42 fake mp4 payload";

//
//
//
//
//
//
//
//
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
            //
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

#
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
    //
    assert_eq!(resp.videos[0].bytes, VEO_VIDEO_BYTES.as_bytes());
    assert!(
        resp.videos[0].url.is_empty(),
        "download delivery must clear url after fetching bytes"
    );
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
}

#
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

const VERTEX_VEO_MODEL: &str = "veo-3.1-generate-preview";

//
//
const VERTEX_VIDEO_BYTES: &str = "fake mp4 video bytes";
const VERTEX_VIDEO_B64: &str = "ZmFrZSBtcDQgdmlkZW8gYnl0ZXM=";

//
//
//
//
//
//
//
fn vertex_exchanges(pending_polls: usize, done_body: serde_json::Value) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains(
                    "POST /veo-3.1-generate-preview:predictLongRunning"
                ),
                "submit must POST the Vertex predictLongRunning endpoint: {request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: bearer test-token"),
                "submit must carry bearer auth: {request}"
            );
            assert!(
                body.get("model").is_none(),
                "Vertex Veo submit body must NOT carry a model field: {body}"
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
            "name": "projects/p/locations/us-central1/operations/op-1"
        })),
    });

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, body: serde_json::Value| {
                assert!(
                    request.contains(
                        "POST /veo-3.1-generate-preview:fetchPredictOperation"
                    ),
                    "poll must POST the fetchPredictOperation endpoint: {request}"
                );
                assert_eq!(
                    body["operationName"].as_str(),
                    Some("projects/p/locations/us-central1/operations/op-1"),
                    "poll body must carry the operationName: {body}"
                );
            }),
            response: json_response(serde_json::json!({ "done": false })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains(
                    "POST /veo-3.1-generate-preview:fetchPredictOperation"
                ),
                "poll must POST the fetchPredictOperation endpoint: {request}"
            );
            assert_eq!(
                body["operationName"].as_str(),
                Some("projects/p/locations/us-central1/operations/op-1"),
                "poll body must carry the operationName: {body}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#
async fn submit_and_wait_vertex_veo_inline_download_delivery() {
    let done = serde_json::json!({
        "done": true,
        "response": {
            "videos": [
                { "bytesBase64Encoded": VERTEX_VIDEO_B64, "mimeType": "video/mp4" }
            ]
        }
    });
    let url = serve_sequence(vertex_exchanges(2, done));

    let mut client = vertex("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(VERTEX_VEO_MODEL)
        .submit("a drone shot over the alps at sunrise")
        .await
        .expect("submit succeeds");
    assert_eq!(
        handle.id,
        "projects/p/locations/us-central1/operations/op-1"
    );
    //
    assert_eq!(handle.model, VERTEX_VEO_MODEL);

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    //
    //
    assert_eq!(resp.videos[0].bytes, VERTEX_VIDEO_BYTES.as_bytes());
    assert!(
        resp.videos[0].url.is_empty(),
        "inline download delivery must leave url empty"
    );
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
}

#
async fn wait_vertex_veo_failed_operation_errors_with_message() {
    let done = serde_json::json!({
        "done": true,
        "error": { "code": 3, "message": "prompt blocked by safety filter" }
    });
    let url = serve_sequence(vertex_exchanges(0, done));

    let mut client = vertex("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(VERTEX_VEO_MODEL)
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

#
async fn wait_vertex_veo_done_no_bytes_errors() {
    //
    //
    let done = serde_json::json!({
        "done": true,
        "response": { "videos": [] }
    });
    let url = serve_sequence(vertex_exchanges(0, done));

    let mut client = vertex("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(VERTEX_VEO_MODEL)
        .submit("a drone shot")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll())
        .await
        .expect_err("done-without-bytes must error");
    assert!(
        err.to_string().contains("no video bytes"),
        "expected a no-bytes error, got: {err}"
    );
}

const NOVA_REEL_MODEL: &str = "amazon.nova-reel-v1:0";
const NOVA_REEL_ARN: &str = "arn:aws:bedrock:us-east-1:123456789012:async-invoke/abc123def456";
//
//
const NOVA_REEL_ARN_ENCODED: &str =
    "arn:aws:bedrock:us-east-1:123456789012:async-invoke%2Fabc123def456";
const NOVA_REEL_OUTPUT_URI: &str = "s3://my-bucket/out/";

//
//
//
//
//
//
//
//
fn bedrock_exchanges(
    pending_polls: usize,
    done_body: serde_json::Value,
    fail_msg: Option<&'static str>,
) -> Vec<TestExchange> {
    let mut exchanges = Vec::new();

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, body: serde_json::Value| {
            assert!(
                request.contains("POST /async-invoke"),
                "submit must POST the start-async-invoke endpoint: {request}"
            );
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("authorization: aws4-hmac-sha256"),
                "submit must carry a SigV4 Authorization header: {request}"
            );
            assert_eq!(
                body["modelId"], NOVA_REEL_MODEL,
                "Nova Reel carries the model in the body, not the URL: {body}"
            );
            assert_eq!(body["modelInput"]["taskType"], "TEXT_VIDEO");
            assert!(
                body["modelInput"]["textToVideoParams"]["text"]
                    .as_str()
                    .map(|s| !s.is_empty())
                    .unwrap_or(false),
                "submit body must carry a non-empty textToVideoParams.text: {body}"
            );
            assert_eq!(
                body["outputDataConfig"]["s3OutputDataConfig"]["s3Uri"],
                NOVA_REEL_OUTPUT_URI,
                "submit body must carry the caller output s3Uri: {body}"
            );
        }),
        response: json_response(serde_json::json!({ "invocationArn": NOVA_REEL_ARN })),
    });

    if let Some(msg) = fail_msg {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains(&format!("GET /async-invoke/{NOVA_REEL_ARN_ENCODED}")),
                    "poll must GET the async-invoke endpoint with the ARN encoded as one segment: {request}"
                );
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("authorization: aws4-hmac-sha256"),
                    "poll must carry a SigV4 Authorization header: {request}"
                );
            }),
            response: json_response(
                serde_json::json!({ "status": "Failed", "failureMessage": msg }),
            ),
        });
        return exchanges;
    }

    for _ in 0..pending_polls {
        exchanges.push(TestExchange {
            assert_request: Box::new(|request: String, _body| {
                assert!(
                    request.contains(&format!("GET /async-invoke/{NOVA_REEL_ARN_ENCODED}")),
                    "poll must GET the async-invoke endpoint with the ARN encoded as one segment: {request}"
                );
            }),
            response: json_response(serde_json::json!({ "status": "InProgress" })),
        });
    }

    exchanges.push(TestExchange {
        assert_request: Box::new(|request: String, _body| {
            assert!(
                request.contains(&format!("GET /async-invoke/{NOVA_REEL_ARN_ENCODED}")),
                "poll must GET the async-invoke endpoint with the ARN encoded as one segment: {request}"
            );
        }),
        response: json_response(done_body),
    });

    exchanges
}

#
async fn submit_and_wait_bedrock_resolves_output_uri_delivery() {
    let done = serde_json::json!({
        "status": "Completed",
        "outputDataConfig": {
            "s3OutputDataConfig": { "s3Uri": NOVA_REEL_OUTPUT_URI }
        }
    });
    let url = serve_sequence(bedrock_exchanges(2, done, None));

    let mut client = bedrock("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(NOVA_REEL_MODEL)
        .output_uri(NOVA_REEL_OUTPUT_URI)
        .submit("a drone shot over the alps, 6s")
        .await
        .expect("submit succeeds");
    assert_eq!(
        handle.id, NOVA_REEL_ARN,
        "handle id must be the invocationArn"
    );

    let resp = wait_video(&handle, fast_poll()).await.expect("wait succeeds");
    assert_eq!(resp.videos.len(), 1);
    //
    //
    assert_eq!(resp.videos[0].url, NOVA_REEL_OUTPUT_URI);
    assert!(
        resp.videos[0].bytes.is_empty(),
        "output-uri delivery must not download bytes"
    );
    assert_eq!(resp.videos[0].mime_type, "video/mp4");
}

#
async fn submit_bedrock_requires_output_uri() {
    //
    //
    let result = bedrock("test-token")
        .video()
        .model(NOVA_REEL_MODEL)
        .submit("a drone shot over the alps")
        .await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "output_uri"),
        other => panic!("expected output_uri validation error, got {:?}", other),
    }
}

#
async fn wait_bedrock_failed_status_surfaces_failure_message() {
    let url = serve_sequence(bedrock_exchanges(
        0,
        serde_json::Value::Null,
        Some("S3 bucket not writable by the service role"),
    ));

    let mut client = bedrock("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(NOVA_REEL_MODEL)
        .output_uri(NOVA_REEL_OUTPUT_URI)
        .submit("a drone shot over the alps")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll())
        .await
        .expect_err("Failed invocation must error");
    assert!(
        err.to_string()
            .contains("S3 bucket not writable by the service role"),
        "error must surface the failureMessage, got: {err}"
    );
}

#
async fn wait_bedrock_completed_no_uri_errors() {
    //
    //
    let url = serve_sequence(bedrock_exchanges(
        0,
        serde_json::json!({ "status": "Completed" }),
        None,
    ));

    let mut client = bedrock("test-token");
    client.provider.base_url = Some(url);

    let handle = client
        .video()
        .model(NOVA_REEL_MODEL)
        .output_uri(NOVA_REEL_OUTPUT_URI)
        .submit("a drone shot")
        .await
        .expect("submit succeeds");

    let err = wait_video(&handle, fast_poll())
        .await
        .expect_err("Completed-without-uri must error");
    assert!(
        err.to_string().contains("no output s3 uri"),
        "expected a no-uri error, got: {err}"
    );
}

#
async fn wait_via_extension_trait_resolves() {
    //
    //
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

#
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

#
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

#
async fn submit_requires_model() {
    let result = grok("test-token").video().submit("no model set").await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "model"),
        other => panic!("expected model validation error, got {:?}", other),
    }
}

#
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

#
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
        headers: std::collections::HashMap::new(),
    }
}

//
//
//
#
async fn submit_rejects_lyrics_part() {
    let req = VideoRequest {
        model: GROK_VIDEO_MODEL.into(),
        prompt: String::new(),
        parts: vec![Part::lyrics("la la la")],
        output_uri: String::new(),
    };
    let result = submit_video(&provider(), &req, &[], false).await;
    match result {
        Err(llmkit::Error::Validation { field, .. }) => assert_eq!(field, "parts"),
        other => panic!("expected parts validation error, got {:?}", other),
    }
}

#
async fn submit_enforces_prompt_parts_xor() {
    //
    let neither = VideoRequest {
        model: GROK_VIDEO_MODEL.into(),
        ..VideoRequest::default()
    };
    assert!(
        submit_video(&provider(), &neither, &[], false).await.is_err(),
        "neither prompt nor parts must error"
    );

    //
    let both = VideoRequest {
        model: GROK_VIDEO_MODEL.into(),
        prompt: "x".into(),
        parts: vec![Part::text("y")],
        output_uri: String::new(),
    };
    assert!(
        submit_video(&provider(), &both, &[], false).await.is_err(),
        "both prompt and parts must error"
    );
}
