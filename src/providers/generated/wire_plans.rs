// Code generated — DO NOT EDIT.


//
//
//
//
//
//
//

///
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FieldBinding {
    pub path: &'static str,
    pub source: &'static str,
    pub option_key: &'static str,
    pub const_json: &'static str,
    pub default_json: &'static str,
    pub transform: &'static str,
    pub omit_if_empty: bool,
}

///
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BodyPlan {
    pub label: &'static str,
    pub bindings: &'static [FieldBinding],
}

pub static PLAN_VIDEO_BEDROCK: BodyPlan = BodyPlan {
    label: "video-bedrock",
    bindings: &[
        FieldBinding { path: "modelId", source: "Model", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "modelInput.taskType", source: "Const", option_key: "", const_json: "\"TEXT_VIDEO\"", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "modelInput.textToVideoParams.text", source: "Prompt", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "outputDataConfig.s3OutputDataConfig.s3Uri", source: "Option", option_key: "output_uri", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
    ],
};

pub static PLAN_VIDEO_GROK: BodyPlan = BodyPlan {
    label: "video-grok",
    bindings: &[
        FieldBinding { path: "model", source: "Model", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "prompt", source: "Prompt", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "image.url", source: "MediaRef", option_key: "", const_json: "", default_json: "", transform: "DataUri", omit_if_empty: true },
    ],
};

pub static PLAN_VIDEO_MODEL_PROMPT: BodyPlan = BodyPlan {
    label: "video-model-prompt",
    bindings: &[
        FieldBinding { path: "model", source: "Model", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "prompt", source: "Prompt", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
    ],
};

pub static PLAN_VIDEO_PIX_VERSE: BodyPlan = BodyPlan {
    label: "video-pixverse",
    bindings: &[
        FieldBinding { path: "model", source: "Model", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "prompt", source: "Prompt", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "duration", source: "Option", option_key: "duration", const_json: "", default_json: "5", transform: "None", omit_if_empty: false },
        FieldBinding { path: "quality", source: "Option", option_key: "quality", const_json: "", default_json: "\"540p\"", transform: "None", omit_if_empty: false },
        FieldBinding { path: "aspect_ratio", source: "Option", option_key: "aspect_ratio", const_json: "", default_json: "\"16:9\"", transform: "None", omit_if_empty: false },
    ],
};

pub static PLAN_VIDEO_QWEN: BodyPlan = BodyPlan {
    label: "video-qwen",
    bindings: &[
        FieldBinding { path: "input.prompt", source: "Prompt", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
        FieldBinding { path: "model", source: "Model", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
    ],
};

pub static PLAN_VIDEO_VEO_INSTANCES: BodyPlan = BodyPlan {
    label: "video-veo-instances",
    bindings: &[
        FieldBinding { path: "instances[0].prompt", source: "Prompt", option_key: "", const_json: "", default_json: "", transform: "None", omit_if_empty: false },
    ],
};

//
pub static VIDEO_BODY_PLANS: &[(&str, &BodyPlan)] = &[
    ("VideoBedrock", &PLAN_VIDEO_BEDROCK),
    ("VideoGrok", &PLAN_VIDEO_GROK),
    ("VideoMinimax", &PLAN_VIDEO_MODEL_PROMPT),
    ("VideoPixVerse", &PLAN_VIDEO_PIX_VERSE),
    ("VideoQwen", &PLAN_VIDEO_QWEN),
    ("VideoTogether", &PLAN_VIDEO_MODEL_PROMPT),
    ("VideoVeo", &PLAN_VIDEO_VEO_INSTANCES),
    ("VideoVertexVeo", &PLAN_VIDEO_VEO_INSTANCES),
    ("VideoVidu", &PLAN_VIDEO_MODEL_PROMPT),
    ("VideoZhipu", &PLAN_VIDEO_MODEL_PROMPT),
];
