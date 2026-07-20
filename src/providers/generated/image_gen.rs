//


use super::providers::ProviderName;

#
pub struct ImageModelDef {
    pub model_id: &'static str,
    pub label: &'static str,
    pub aspect_ratios: &'static [&'static str],
    pub image_sizes: &'static [&'static str],
    ///
    ///
    ///
    pub max_input_images: i64,
}

#
pub struct ImageGenDef {
    pub input_mode: &'static str,
    pub output_mode: &'static str,
    ///
    pub response_shape: &'static str,
    ///
    pub usage_input_path: &'static str,
    pub usage_output_path: &'static str,
    pub max_input_count: usize,
    pub gen_endpoint: &'static str,
    pub edit_endpoint: &'static str,
    pub models: &'static [ImageModelDef],
}

static GOOGLE_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "gemini-3-pro-image-preview",
        label: "Nano Banana Pro",
        aspect_ratios: &["16:9", "1:1", "21:9", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16"],
        image_sizes: &["1K", "2K", "4K"],
        max_input_images: 0,
    },
    ImageModelDef {
        model_id: "gemini-3.1-flash-image-preview",
        label: "Nano Banana 2",
        aspect_ratios: &["16:9", "1:1", "1:4", "1:8", "21:9", "2:3", "3:2", "3:4", "4:1", "4:3", "4:5", "5:4", "8:1", "9:16"],
        image_sizes: &["1K", "2K", "4K", "512"],
        max_input_images: 0,
    },
];

static GOOGLE_IMAGE_GEN: ImageGenDef = ImageGenDef {
    input_mode: "InlineParts",
    output_mode: "Base64Inline",
    response_shape: "GoogleParts",
    usage_input_path: "usageMetadata.promptTokenCount",
    usage_output_path: "usageMetadata.candidatesTokenCount",
    max_input_count: 14,
    gen_endpoint: "",
    edit_endpoint: "",
    models: GOOGLE_IMAGE_MODELS,
};

static GROK_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "grok-imagine-image-quality",
        label: "Grok Imagine Quality",
        aspect_ratios: &["16:9", "19.5:9", "1:1", "1:2", "20:9", "2:1", "2:3", "3:2", "3:4", "4:3", "9:16", "9:19.5", "9:20", "auto"],
        image_sizes: &[],
        max_input_images: 0,
    },
];

static GROK_IMAGE_GEN: ImageGenDef = ImageGenDef {
    input_mode: "JSONInlineRefs",
    output_mode: "Base64Inline",
    response_shape: "DataArrayB64Json",
    usage_input_path: "",
    usage_output_path: "",
    max_input_count: 16,
    gen_endpoint: "/v1/images/generations",
    edit_endpoint: "/v1/images/edits",
    models: GROK_IMAGE_MODELS,
};

static OPENAI_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "gpt-image-1",
        label: "GPT Image 1",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
    },
    ImageModelDef {
        model_id: "gpt-image-1-mini",
        label: "GPT Image 1 Mini",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
    },
    ImageModelDef {
        model_id: "gpt-image-1.5",
        label: "GPT Image 1.5",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
    },
    ImageModelDef {
        model_id: "gpt-image-2",
        label: "GPT Image 2",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
    },
];

static OPENAI_IMAGE_GEN: ImageGenDef = ImageGenDef {
    input_mode: "MultipartForm",
    output_mode: "Base64Inline",
    response_shape: "DataArrayB64Json",
    usage_input_path: "usage.input_tokens",
    usage_output_path: "usage.output_tokens",
    max_input_count: 16,
    gen_endpoint: "/v1/images/generations",
    edit_endpoint: "/v1/images/edits",
    models: OPENAI_IMAGE_MODELS,
};

static RECRAFT_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "recraftv3",
        label: "Recraft V3",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
    },
    ImageModelDef {
        model_id: "recraftv3_vector",
        label: "Recraft V3 (vector / SVG)",
        aspect_ratios: &[],
        image_sizes: &[],
        max_input_images: 0,
    },
];

static RECRAFT_IMAGE_GEN: ImageGenDef = ImageGenDef {
    input_mode: "JSONGenerations",
    output_mode: "Base64Inline",
    response_shape: "DataArrayB64Json",
    usage_input_path: "",
    usage_output_path: "",
    max_input_count: 0,
    gen_endpoint: "/v1/images/generations",
    edit_endpoint: "",
    models: RECRAFT_IMAGE_MODELS,
};

static VERTEX_IMAGE_MODELS: &[ImageModelDef] = &[
    ImageModelDef {
        model_id: "imagen-3.0-fast-generate-001",
        label: "Imagen 3 Fast",
        aspect_ratios: &["16:9", "1:1", "3:4", "4:3", "9:16"],
        image_sizes: &[],
        max_input_images: 0,
    },
    ImageModelDef {
        model_id: "imagen-3.0-generate-002",
        label: "Imagen 3",
        aspect_ratios: &["16:9", "1:1", "3:4", "4:3", "9:16"],
        image_sizes: &[],
        max_input_images: 0,
    },
    ImageModelDef {
        model_id: "imagen-4.0-generate-preview-06-06",
        label: "Imagen 4 Preview",
        aspect_ratios: &["16:9", "1:1", "3:4", "4:3", "9:16"],
        image_sizes: &[],
        max_input_images: 0,
    },
];

static VERTEX_IMAGE_GEN: ImageGenDef = ImageGenDef {
    input_mode: "JSONPredict",
    output_mode: "Base64Inline",
    response_shape: "VertexPredictions",
    usage_input_path: "",
    usage_output_path: "",
    max_input_count: 1,
    gen_endpoint: "",
    edit_endpoint: "",
    models: VERTEX_IMAGE_MODELS,
};

pub fn image_gen_config(provider: ProviderName) -> Option<&'static ImageGenDef> {
    match provider {
        ProviderName::Google => Some(&GOOGLE_IMAGE_GEN),
        ProviderName::Grok => Some(&GROK_IMAGE_GEN),
        ProviderName::OpenAI => Some(&OPENAI_IMAGE_GEN),
        ProviderName::Recraft => Some(&RECRAFT_IMAGE_GEN),
        ProviderName::Vertex => Some(&VERTEX_IMAGE_GEN),
        _ => None,
    }
}
