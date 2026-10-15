// Code generated — DO NOT EDIT.

use super::providers::ProviderName;

/// Carried input: a request variant that carries text only (ADR-096).
pub const TEXT_ONLY: &str = "TextOnly";
/// Carried input: a request variant that carries reference images (ADR-096).
pub const WITH_REFERENCE_IMAGES: &str = "WithReferenceImages";
/// Carried input: a video request variant that carries a seed frame (ADR-096).
pub const WITH_SEED_FRAME: &str = "WithSeedFrame";
/// Carried input: a video request variant that carries a last frame (ADR-096).
pub const WITH_LAST_FRAME: &str = "WithLastFrame";

// Image request shapes: each names one request body (ADR-096).
pub const INLINE_PARTS: &str = "InlineParts";
pub const JSON_GENERATIONS_OPENAI: &str = "JSONGenerationsOpenAI";
pub const MULTIPART_FORM: &str = "MultipartForm";
pub const JSON_INLINE_REFS: &str = "JSONInlineRefs";
/// Deprecated: no provider accepts this since 2026-10-15 (Vertex Imagen shut down, BUG-101); removed at the next major.
pub const JSON_PREDICT: &str = "JSONPredict";
pub const JSON_GENERATIONS: &str = "JSONGenerations";

/// One wire request of an image or video config, keyed by the inputs it
/// carries (ADR-096). `shape` is empty on a video variant; an empty
/// `endpoint` means the provider's main endpoint template. `carries` lists
/// the carried inputs; a first-and-last-frame variant carries two.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestVariant {
    pub carries: &'static [&'static str],
    pub shape: &'static str,
    pub endpoint: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageModelDef {
    pub model_id: &'static str,
    pub label: &'static str,
    pub aspect_ratios: &'static [&'static str],
    pub image_sizes: &'static [&'static str],
    /// Images llmkit serializes when the wire shape fixes the count
    /// (e.g. Grok's single-seed slot); 0 = no llmkit limit, the provider
    /// decides volume (BUG-011).
    pub max_input_images: i64,
    /// Whether llmkit sends Image Parts for this model; false means the
    /// runtime rejects them (BUG-096).
    pub supports_image_input: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageGenDef {
    pub output_mode: &'static str,
    /// Response wire family selecting the response parser (BUG-024).
    pub response_shape: &'static str,
    /// Dotted-from-root usage-token paths; empty when unreported.
    pub usage_input_path: &'static str,
    pub usage_output_path: &'static str,
    pub usage_reasoning_path: &'static str,
    /// One request per carried input, TextOnly first (ADR-096).
    pub variants: &'static [RequestVariant],
    pub models: &'static [ImageModelDef],
}

static GOOGLE_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "gemini-3-pro-image-preview",
        label: "Nano Banana Pro",
        aspect_ratios: &[
            "16:9", "1:1", "21:9", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16",
        ],
        image_sizes: &["1K", "2K", "4K"],
        max_input_images: 0,
        supports_image_input: true,
    },
    ImageModelDef {
        model_id: "gemini-3.1-flash-image-preview",
        label: "Nano Banana 2",
        aspect_ratios: &[
            "16:9", "1:1", "1:4", "1:8", "21:9", "2:3", "3:2", "3:4", "4:1", "4:3", "4:5", "5:4",
            "8:1", "9:16",
        ],
        image_sizes: &["1K", "2K", "4K", "512"],
        max_input_images: 0,
        supports_image_input: true,
    },
];

static GOOGLE_IMAGE_GEN: ImageGenDef = ImageGenDef {
    output_mode: "Base64Inline",
    response_shape: "GoogleParts",
    usage_input_path: "usageMetadata.promptTokenCount",
    usage_output_path: "usageMetadata.candidatesTokenCount",
    usage_reasoning_path: "usageMetadata.thoughtsTokenCount",
    variants: &[
        RequestVariant {
            carries: &["TextOnly"],
            shape: "InlineParts",
            endpoint: "",
        },
        RequestVariant {
            carries: &["WithReferenceImages"],
            shape: "InlineParts",
            endpoint: "",
        },
    ],
    models: GOOGLE_IMAGE_MODELS,
};

static GROK_IMAGE_MODELS: &[ImageModelDef] = &[ImageModelDef {
    model_id: "grok-imagine-image-quality",
    label: "Grok Imagine Quality",
    aspect_ratios: &[
        "16:9", "19.5:9", "1:1", "1:2", "20:9", "2:1", "2:3", "3:2", "3:4", "4:3", "9:16",
        "9:19.5", "9:20", "auto",
    ],
    image_sizes: &[],
    max_input_images: 0,
    supports_image_input: true,
}];

static GROK_IMAGE_GEN: ImageGenDef = ImageGenDef {
    output_mode: "Base64Inline",
    response_shape: "DataArrayB64Json",
    usage_input_path: "",
    usage_output_path: "",
    usage_reasoning_path: "",
    variants: &[
        RequestVariant {
            carries: &["TextOnly"],
            shape: "JSONInlineRefs",
            endpoint: "/v1/images/generations",
        },
        RequestVariant {
            carries: &["WithReferenceImages"],
            shape: "JSONInlineRefs",
            endpoint: "/v1/images/edits",
        },
    ],
    models: GROK_IMAGE_MODELS,
};

static OPENAI_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "gpt-image-1",
        label: "GPT Image 1",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: true,
    },
    ImageModelDef {
        model_id: "gpt-image-1-mini",
        label: "GPT Image 1 Mini",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: true,
    },
    ImageModelDef {
        model_id: "gpt-image-1.5",
        label: "GPT Image 1.5",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: true,
    },
    ImageModelDef {
        model_id: "gpt-image-2",
        label: "GPT Image 2",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: true,
    },
];

static OPENAI_IMAGE_GEN: ImageGenDef = ImageGenDef {
    output_mode: "Base64Inline",
    response_shape: "DataArrayB64Json",
    usage_input_path: "usage.input_tokens",
    usage_output_path: "usage.output_tokens",
    usage_reasoning_path: "",
    variants: &[
        RequestVariant {
            carries: &["TextOnly"],
            shape: "JSONGenerationsOpenAI",
            endpoint: "/v1/images/generations",
        },
        RequestVariant {
            carries: &["WithReferenceImages"],
            shape: "MultipartForm",
            endpoint: "/v1/images/edits",
        },
    ],
    models: OPENAI_IMAGE_MODELS,
};

static OPENROUTER_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "google/gemini-2.5-flash-image",
        label: "Nano Banana (Gemini 2.5 Flash Image)",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
    ImageModelDef {
        model_id: "google/gemini-3-pro-image",
        label: "Nano Banana Pro (Gemini 3 Pro Image)",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
    ImageModelDef {
        model_id: "google/gemini-3.1-flash-image",
        label: "Nano Banana 2 (Gemini 3.1 Flash Image)",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
    ImageModelDef {
        model_id: "google/gemini-3.1-flash-lite-image",
        label: "Nano Banana 2 Lite (Gemini 3.1 Flash Lite Image)",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
    ImageModelDef {
        model_id: "openai/gpt-5-image",
        label: "GPT-5 Image",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
    ImageModelDef {
        model_id: "openai/gpt-5-image-mini",
        label: "GPT-5 Image Mini",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
    ImageModelDef {
        model_id: "openai/gpt-5.4-image-2",
        label: "GPT-5.4 Image 2",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
];

static OPENROUTER_IMAGE_GEN: ImageGenDef = ImageGenDef {
    output_mode: "Base64Inline",
    response_shape: "DataArrayB64Json",
    usage_input_path: "usage.prompt_tokens",
    usage_output_path: "usage.completion_tokens",
    usage_reasoning_path: "",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "JSONGenerations",
        endpoint: "/v1/images",
    }],
    models: OPENROUTER_IMAGE_MODELS,
};

static RECRAFT_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "recraftv3",
        label: "Recraft V3",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
    ImageModelDef {
        model_id: "recraftv3_vector",
        label: "Recraft V3 (vector / SVG)",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
        supports_image_input: false,
    },
];

static RECRAFT_IMAGE_GEN: ImageGenDef = ImageGenDef {
    output_mode: "Base64Inline",
    response_shape: "DataArrayB64Json",
    usage_input_path: "",
    usage_output_path: "",
    usage_reasoning_path: "",
    variants: &[RequestVariant {
        carries: &["TextOnly"],
        shape: "JSONGenerations",
        endpoint: "/v1/images/generations",
    }],
    models: RECRAFT_IMAGE_MODELS,
};

pub fn image_gen_config(provider: ProviderName) -> Option<&'static ImageGenDef> {
    match provider {
        ProviderName::Google => Some(&GOOGLE_IMAGE_GEN),
        ProviderName::Grok => Some(&GROK_IMAGE_GEN),
        ProviderName::OpenAI => Some(&OPENAI_IMAGE_GEN),
        ProviderName::Openrouter => Some(&OPENROUTER_IMAGE_GEN),
        ProviderName::Recraft => Some(&RECRAFT_IMAGE_GEN),
        _ => None,
    }
}
