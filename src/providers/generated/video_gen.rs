// Code generated — DO NOT EDIT.

use super::image_gen::RequestVariant;
use super::providers::ProviderName;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoModelDef {
    pub model_id: &'static str,
    pub label: &'static str,
    pub supports_image_to_video: bool,
    /// Whether `last_frame` and `add_reference_image` are accepted; false
    /// means the request is rejected before send (BUG-089).
    pub supports_last_frame: bool,
    pub supports_reference_images: bool,
    pub max_duration_seconds: i64,
    pub output_mime: &'static str,
    pub resolutions: &'static [&'static str],
    /// Accepted aspect ratios (e.g. "16:9"); empty = no fixed set.
    pub aspect_ratios: &'static [&'static str],
    /// Images llmkit serializes when the wire shape fixes the count
    /// (e.g. Grok's single-seed slot); 0 = no llmkit limit, the provider
    /// decides volume (BUG-011).
    pub max_input_images: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VideoGenDef {
    // wire_shape is VideoGrok | VideoZhipu | VideoTogether | VideoQwen | VideoMinimax | VideoBedrock | VideoVertexVeo | VideoVidu | VideoPixVerse | VideoOpenRouter.
    pub wire_shape: &'static str,
    // One of download | url | output-uri (as DeliveryDownload/URL/OutputURI).
    pub output_delivery: &'static str,
    // video_base_url is the video API base when it differs from the chat base; "" = use chat base.
    pub video_base_url: &'static str,
    // variants lists one submit request per carried input, TextOnly first;
    // each endpoint is relative to the resolved video base (ADR-096).
    pub variants: &'static [RequestVariant],
    // poll_endpoint is the poll template with an {id} placeholder.
    pub poll_endpoint: &'static str,
    // file_endpoint is the file-retrieve template with {file_id} for two-hop providers; "" = single-hop.
    pub file_endpoint: &'static str,
    // cancel_endpoint is the cancel template with {model} and {operation_id}; "" = no documented cancel.
    pub cancel_endpoint: &'static str,
    // submit_handle_field is a dotted path to the poll handle id.
    pub submit_handle_field: &'static str,
    pub requires_output_uri: bool,
    pub models: &'static [VideoModelDef],
}

static BEDROCK_VIDEO_MODELS: &[VideoModelDef] = &[VideoModelDef {
    model_id: "amazon.nova-reel-v1:0",
    label: "Nova Reel",
    supports_image_to_video: false,
    supports_last_frame: false,
    supports_reference_images: false,
    max_duration_seconds: 6,
    output_mime: "video/mp4",
    resolutions: &["720p"],
    aspect_ratios: &[],
    max_input_images: 0,
}];

static BEDROCK_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoBedrock",
    output_delivery: "DeliveryOutputURI",
    video_base_url: "",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "",
        endpoint: "/async-invoke",
    }],
    poll_endpoint: "/async-invoke/{id}",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "invocationArn",
    requires_output_uri: true,
    models: BEDROCK_VIDEO_MODELS,
};

static GROK_VIDEO_MODELS: &[VideoModelDef] = &[
    VideoModelDef {
        model_id: "grok-imagine-video",
        label: "Grok Imagine Video",
        supports_image_to_video: true,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["480p", "720p"],
        aspect_ratios: &[
            "16:9", "1:1", "21:9", "2:3", "3:2", "3:4", "4:3", "5:2", "9:16",
        ],
        max_input_images: 1,
    },
    VideoModelDef {
        model_id: "grok-imagine-video-1.5",
        label: "Grok Imagine Video 1.5",
        supports_image_to_video: true,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["1080p", "480p", "720p"],
        aspect_ratios: &[
            "16:9", "1:1", "21:9", "2:3", "3:2", "3:4", "4:3", "5:2", "9:16",
        ],
        max_input_images: 1,
    },
    VideoModelDef {
        model_id: "grok-imagine-video-1.5-lite",
        label: "Grok Imagine Video 1.5 Lite",
        supports_image_to_video: true,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["1080p", "480p", "720p"],
        aspect_ratios: &[
            "16:9", "1:1", "21:9", "2:3", "3:2", "3:4", "4:3", "5:2", "9:16",
        ],
        max_input_images: 1,
    },
];

static GROK_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoGrok",
    output_delivery: "DeliveryURL",
    video_base_url: "",
    variants: &[
        RequestVariant {
            carries: &["TextOnly"],
            shape: "",
            endpoint: "/v1/videos/generations",
        },
        RequestVariant {
            carries: &["WithSeedFrame"],
            shape: "",
            endpoint: "/v1/videos/generations",
        },
    ],
    poll_endpoint: "/v1/videos/{id}",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "request_id",
    requires_output_uri: false,
    models: GROK_VIDEO_MODELS,
};

static MINIMAX_VIDEO_MODELS: &[VideoModelDef] = &[VideoModelDef {
    model_id: "MiniMax-Hailuo-2.3",
    label: "MiniMax Hailuo 2.3",
    supports_image_to_video: false,
    supports_last_frame: false,
    supports_reference_images: false,
    max_duration_seconds: 6,
    output_mime: "video/mp4",
    resolutions: &["1080p", "768p"],
    aspect_ratios: &[],
    max_input_images: 0,
}];

static MINIMAX_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoMinimax",
    output_delivery: "DeliveryURL",
    video_base_url: "https://api.minimax.io",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "",
        endpoint: "/v1/video_generation",
    }],
    poll_endpoint: "/v1/query/video_generation?task_id={id}",
    file_endpoint: "/v1/files/retrieve?file_id={file_id}",
    cancel_endpoint: "",
    submit_handle_field: "task_id",
    requires_output_uri: false,
    models: MINIMAX_VIDEO_MODELS,
};

static OPENROUTER_VIDEO_MODELS: &[VideoModelDef] = &[
    VideoModelDef {
        model_id: "alibaba/wan-3.0",
        label: "Wan 3.0",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 30,
        output_mime: "video/mp4",
        resolutions: &["1080p", "480p", "720p"],
        aspect_ratios: &["16:9", "1:1", "3:4", "4:3", "9:16"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "bytedance/seedance-2.0",
        label: "Seedance 2.0",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["1080p", "480p", "4K", "720p"],
        aspect_ratios: &["16:9", "1:1", "21:9", "3:4", "4:3", "9:16", "9:21"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "bytedance/seedance-2.0-fast",
        label: "Seedance 2.0 Fast",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["480p", "720p"],
        aspect_ratios: &["16:9", "1:1", "21:9", "3:4", "4:3", "9:16", "9:21"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "bytedance/seedance-2.5",
        label: "Seedance 2.5",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 30,
        output_mime: "video/mp4",
        resolutions: &["480p", "720p"],
        aspect_ratios: &["16:9", "1:1", "21:9", "3:4", "4:3", "9:16"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "google/veo-3.1-fast",
        label: "Veo 3.1 Fast",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &["1080p", "4K", "720p"],
        aspect_ratios: &["16:9", "9:16"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "google/veo-3.1-lite",
        label: "Veo 3.1 Lite",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &["1080p", "720p"],
        aspect_ratios: &["16:9", "9:16"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "heygen/heygen-video-1",
        label: "HeyGen Video 1",
        supports_image_to_video: true,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["2K", "480p", "768p"],
        aspect_ratios: &["16:9", "1:1", "21:9", "3:4", "4:3", "9:16"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "kwaivgi/kling-v3.0-std",
        label: "Kling 3.0 Standard",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["720p"],
        aspect_ratios: &["16:9", "1:1", "9:16"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "minimax/hailuo-3",
        label: "Hailuo 3",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["2K"],
        aspect_ratios: &["16:9", "1:1", "21:9", "3:4", "4:3", "9:16"],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "x-ai/grok-imagine-video-1.5-lite",
        label: "Grok Imagine Video 1.5 Lite",
        supports_image_to_video: true,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &["1080p", "480p", "720p"],
        aspect_ratios: &["16:9", "1:1", "2:3", "3:2", "3:4", "4:3", "9:16"],
        max_input_images: 0,
    },
];

static OPENROUTER_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoOpenRouter",
    output_delivery: "DeliveryDownload",
    video_base_url: "",
    variants: &[
        RequestVariant {
            carries: &["TextOnly"],
            shape: "",
            endpoint: "/v1/videos",
        },
        RequestVariant {
            carries: &["WithSeedFrame"],
            shape: "",
            endpoint: "/v1/videos",
        },
        RequestVariant {
            carries: &["WithSeedFrame", "WithLastFrame"],
            shape: "",
            endpoint: "/v1/videos",
        },
    ],
    poll_endpoint: "/v1/videos/{id}",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "id",
    requires_output_uri: false,
    models: OPENROUTER_VIDEO_MODELS,
};

static PIXVERSE_VIDEO_MODELS: &[VideoModelDef] = &[
    VideoModelDef {
        model_id: "v4.5",
        label: "PixVerse v4.5",
        supports_image_to_video: false,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &[],
        aspect_ratios: &[],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "v5",
        label: "PixVerse v5",
        supports_image_to_video: false,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &[],
        aspect_ratios: &[],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "v6",
        label: "PixVerse v6",
        supports_image_to_video: false,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 15,
        output_mime: "video/mp4",
        resolutions: &[],
        aspect_ratios: &[],
        max_input_images: 0,
    },
];

static PIXVERSE_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoPixVerse",
    output_delivery: "DeliveryURL",
    video_base_url: "",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "",
        endpoint: "/openapi/v2/video/text/generate",
    }],
    poll_endpoint: "/openapi/v2/video/result/{id}",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "Resp.video_id",
    requires_output_uri: false,
    models: PIXVERSE_VIDEO_MODELS,
};

static QWEN_VIDEO_MODELS: &[VideoModelDef] = &[VideoModelDef {
    model_id: "wan2.2-t2v-plus",
    label: "Wan 2.2 T2V Plus",
    supports_image_to_video: false,
    supports_last_frame: false,
    supports_reference_images: false,
    max_duration_seconds: 5,
    output_mime: "video/mp4",
    resolutions: &["720p"],
    aspect_ratios: &[],
    max_input_images: 0,
}];

static QWEN_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoQwen",
    output_delivery: "DeliveryURL",
    video_base_url: "https://dashscope-intl.aliyuncs.com",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "",
        endpoint: "/api/v1/services/aigc/video-generation/video-synthesis",
    }],
    poll_endpoint: "/api/v1/tasks/{id}",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "output.task_id",
    requires_output_uri: false,
    models: QWEN_VIDEO_MODELS,
};

static TOGETHER_VIDEO_MODELS: &[VideoModelDef] = &[VideoModelDef {
    model_id: "minimax/video-01-director",
    label: "MiniMax Video 01 Director (Together)",
    supports_image_to_video: false,
    supports_last_frame: false,
    supports_reference_images: false,
    max_duration_seconds: 6,
    output_mime: "video/mp4",
    resolutions: &["720p"],
    aspect_ratios: &[],
    max_input_images: 0,
}];

static TOGETHER_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoTogether",
    output_delivery: "DeliveryURL",
    video_base_url: "",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "",
        endpoint: "/v2/videos",
    }],
    poll_endpoint: "/v2/videos/{id}",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "id",
    requires_output_uri: false,
    models: TOGETHER_VIDEO_MODELS,
};

static VERTEX_VIDEO_MODELS: &[VideoModelDef] = &[
    VideoModelDef {
        model_id: "veo-3.1-fast-generate-001",
        label: "Veo 3.1 Fast",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: true,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &["1080p", "720p"],
        aspect_ratios: &["16:9", "9:16"],
        max_input_images: 1,
    },
    VideoModelDef {
        model_id: "veo-3.1-generate-001",
        label: "Veo 3.1",
        supports_image_to_video: true,
        supports_last_frame: true,
        supports_reference_images: true,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &["1080p", "4k", "720p"],
        aspect_ratios: &["16:9", "9:16"],
        max_input_images: 1,
    },
    VideoModelDef {
        model_id: "veo-3.1-generate-preview",
        label: "Veo 3.1",
        supports_image_to_video: true,
        supports_last_frame: false,
        supports_reference_images: true,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &["1080p", "720p"],
        aspect_ratios: &["16:9", "9:16"],
        max_input_images: 1,
    },
    VideoModelDef {
        model_id: "veo-3.1-lite-generate-001",
        label: "Veo 3.1 Lite",
        supports_image_to_video: false,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 8,
        output_mime: "video/mp4",
        resolutions: &["1080p", "720p"],
        aspect_ratios: &["16:9", "9:16"],
        max_input_images: 0,
    },
];

static VERTEX_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoVertexVeo",
    output_delivery: "DeliveryDownload",
    video_base_url: "",
    variants: &[
        RequestVariant {
            carries: &["TextOnly"],
            shape: "",
            endpoint: "/{model}:predictLongRunning",
        },
        RequestVariant {
            carries: &["WithReferenceImages"],
            shape: "",
            endpoint: "/{model}:predictLongRunning",
        },
        RequestVariant {
            carries: &["WithSeedFrame"],
            shape: "",
            endpoint: "/{model}:predictLongRunning",
        },
        RequestVariant {
            carries: &["WithSeedFrame", "WithLastFrame"],
            shape: "",
            endpoint: "/{model}:predictLongRunning",
        },
    ],
    poll_endpoint: "/{model}:fetchPredictOperation",
    file_endpoint: "",
    cancel_endpoint: "/{model}/operations/{operation_id}:cancel",
    submit_handle_field: "name",
    requires_output_uri: false,
    models: VERTEX_VIDEO_MODELS,
};

static VIDU_VIDEO_MODELS: &[VideoModelDef] = &[
    VideoModelDef {
        model_id: "viduq1",
        label: "Vidu Q1",
        supports_image_to_video: false,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 16,
        output_mime: "video/mp4",
        resolutions: &[],
        aspect_ratios: &[],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "viduq2",
        label: "Vidu Q2",
        supports_image_to_video: false,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 16,
        output_mime: "video/mp4",
        resolutions: &[],
        aspect_ratios: &[],
        max_input_images: 0,
    },
    VideoModelDef {
        model_id: "viduq3-pro",
        label: "Vidu Q3 Pro",
        supports_image_to_video: false,
        supports_last_frame: false,
        supports_reference_images: false,
        max_duration_seconds: 16,
        output_mime: "video/mp4",
        resolutions: &[],
        aspect_ratios: &[],
        max_input_images: 0,
    },
];

static VIDU_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoVidu",
    output_delivery: "DeliveryURL",
    video_base_url: "",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "",
        endpoint: "/ent/v2/text2video",
    }],
    poll_endpoint: "/ent/v2/tasks/{id}/creations",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "task_id",
    requires_output_uri: false,
    models: VIDU_VIDEO_MODELS,
};

static ZHIPU_VIDEO_MODELS: &[VideoModelDef] = &[VideoModelDef {
    model_id: "cogvideox-3",
    label: "CogVideoX-3",
    supports_image_to_video: false,
    supports_last_frame: false,
    supports_reference_images: false,
    max_duration_seconds: 10,
    output_mime: "video/mp4",
    resolutions: &["1080p", "4k", "720p"],
    aspect_ratios: &[],
    max_input_images: 0,
}];

static ZHIPU_VIDEO_GEN: VideoGenDef = VideoGenDef {
    wire_shape: "VideoZhipu",
    output_delivery: "DeliveryURL",
    video_base_url: "",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "",
        endpoint: "/v4/videos/generations",
    }],
    poll_endpoint: "/v4/async-result/{id}",
    file_endpoint: "",
    cancel_endpoint: "",
    submit_handle_field: "id",
    requires_output_uri: false,
    models: ZHIPU_VIDEO_MODELS,
};

pub fn video_gen_config(provider: ProviderName) -> Option<&'static VideoGenDef> {
    match provider {
        ProviderName::Bedrock => Some(&BEDROCK_VIDEO_GEN),
        ProviderName::Grok => Some(&GROK_VIDEO_GEN),
        ProviderName::Minimax => Some(&MINIMAX_VIDEO_GEN),
        ProviderName::Openrouter => Some(&OPENROUTER_VIDEO_GEN),
        ProviderName::Pixverse => Some(&PIXVERSE_VIDEO_GEN),
        ProviderName::Qwen => Some(&QWEN_VIDEO_GEN),
        ProviderName::Together => Some(&TOGETHER_VIDEO_GEN),
        ProviderName::Vertex => Some(&VERTEX_VIDEO_GEN),
        ProviderName::Vidu => Some(&VIDU_VIDEO_GEN),
        ProviderName::Zhipu => Some(&ZHIPU_VIDEO_GEN),
        _ => None,
    }
}
