//


//!
//!
//!

use crate::providers::generated::providers::ProviderName;
use crate::types::Capability;

///
///
///
#
pub struct CompiledModelDef {
    pub id: &'static str,
    pub provider: ProviderName,
    pub capabilities: &'static [Capability],
    pub display_name: &'static str,
    pub description: &'static str,
    pub context_window: i64,
    pub max_output: i64,
}

pub static COMPILED_IN_MODELS: &[CompiledModelDef] = &[
    CompiledModelDef {
        id: "claude-haiku-4-5-20251001",
        provider: ProviderName::Anthropic,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "Claude Haiku 4.5",
        description: "",
        context_window: 200000,
        max_output: 64000,
    },
    CompiledModelDef {
        id: "claude-opus-4-5-20251101",
        provider: ProviderName::Anthropic,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "Claude Opus 4.5",
        description: "",
        context_window: 200000,
        max_output: 64000,
    },
    CompiledModelDef {
        id: "claude-opus-4-6",
        provider: ProviderName::Anthropic,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "Claude Opus 4.6",
        description: "",
        context_window: 1000000,
        max_output: 128000,
    },
    CompiledModelDef {
        id: "claude-opus-4-7",
        provider: ProviderName::Anthropic,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "Claude Opus 4.7",
        description: "",
        context_window: 1000000,
        max_output: 128000,
    },
    CompiledModelDef {
        id: "claude-sonnet-4-5-20250929",
        provider: ProviderName::Anthropic,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "Claude Sonnet 4.5",
        description: "",
        context_window: 1000000,
        max_output: 64000,
    },
    CompiledModelDef {
        id: "claude-sonnet-4-6",
        provider: ProviderName::Anthropic,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "Claude Sonnet 4.6",
        description: "",
        context_window: 1000000,
        max_output: 128000,
    },
    CompiledModelDef {
        id: "gemini-2.5-flash",
        provider: ProviderName::Google,
        capabilities: &[Capability::ChatCompletion, Capability::Reasoning, Capability::ToolCalling],
        display_name: "Gemini 2.5 Flash",
        description: "Stable version of Gemini 2.5 Flash",
        context_window: 1048576,
        max_output: 65536,
    },
    CompiledModelDef {
        id: "gemini-2.5-flash-lite",
        provider: ProviderName::Google,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "Gemini 2.5 Flash-Lite",
        description: "Stable version of Gemini 2.5 Flash-Lite",
        context_window: 1048576,
        max_output: 65536,
    },
    CompiledModelDef {
        id: "gemini-2.5-pro",
        provider: ProviderName::Google,
        capabilities: &[Capability::ChatCompletion, Capability::Reasoning, Capability::ToolCalling],
        display_name: "Gemini 2.5 Pro",
        description: "Stable release of Gemini 2.5 Pro",
        context_window: 1048576,
        max_output: 65536,
    },
    CompiledModelDef {
        id: "gemini-3-pro-image-preview",
        provider: ProviderName::Google,
        capabilities: &[Capability::ImageGeneration],
        display_name: "Nano Banana Pro",
        description: "Gemini 3 Pro Image Preview",
        context_window: 131072,
        max_output: 32768,
    },
    CompiledModelDef {
        id: "gemini-3-pro-preview",
        provider: ProviderName::Google,
        capabilities: &[Capability::ChatCompletion, Capability::Reasoning, Capability::ToolCalling],
        display_name: "Gemini 3 Pro Preview",
        description: "Gemini 3 Pro Preview",
        context_window: 1048576,
        max_output: 65536,
    },
    CompiledModelDef {
        id: "gemini-3.1-flash-image-preview",
        provider: ProviderName::Google,
        capabilities: &[Capability::ImageGeneration],
        display_name: "Nano Banana 2",
        description: "Gemini 3.1 Flash Image Preview",
        context_window: 65536,
        max_output: 65536,
    },
    CompiledModelDef {
        id: "gpt-4o",
        provider: ProviderName::OpenAI,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "",
        description: "",
        context_window: 0,
        max_output: 0,
    },
    CompiledModelDef {
        id: "gpt-4o-mini",
        provider: ProviderName::OpenAI,
        capabilities: &[Capability::ChatCompletion, Capability::ToolCalling],
        display_name: "",
        description: "",
        context_window: 0,
        max_output: 0,
    },
    CompiledModelDef {
        id: "gpt-5",
        provider: ProviderName::OpenAI,
        capabilities: &[Capability::ChatCompletion, Capability::Reasoning, Capability::ToolCalling],
        display_name: "",
        description: "",
        context_window: 0,
        max_output: 0,
    },
    CompiledModelDef {
        id: "gpt-image-1",
        provider: ProviderName::OpenAI,
        capabilities: &[Capability::ImageGeneration],
        display_name: "",
        description: "",
        context_window: 0,
        max_output: 0,
    },
    CompiledModelDef {
        id: "o1",
        provider: ProviderName::OpenAI,
        capabilities: &[Capability::ChatCompletion, Capability::Reasoning],
        display_name: "",
        description: "",
        context_window: 0,
        max_output: 0,
    },
    CompiledModelDef {
        id: "o3",
        provider: ProviderName::OpenAI,
        capabilities: &[Capability::ChatCompletion, Capability::Reasoning, Capability::ToolCalling],
        display_name: "",
        description: "",
        context_window: 0,
        max_output: 0,
    },
    CompiledModelDef {
        id: "o4-mini",
        provider: ProviderName::OpenAI,
        capabilities: &[Capability::ChatCompletion, Capability::Reasoning, Capability::ToolCalling],
        display_name: "",
        description: "",
        context_window: 0,
        max_output: 0,
    },
];

///
///
///
///
#
pub(crate) fn ontology_capabilities(
    provider: ProviderName,
    model_id: &str,
) -> Option<&'static [Capability]> {
    for m in COMPILED_IN_MODELS {
        if m.provider == provider && m.id == model_id {
            return Some(m.capabilities);
        }
    }
    None
}

///
///
///
#
pub struct CatalogueConfig {
    pub endpoint: &'static str,
    pub pagination: &'static str,
    pub cursor_param: &'static str,
    pub parser_kind: &'static str,
    pub spec_url: &'static str,
    pub spec_format: &'static str,
}

static ANTHROPIC_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "CursorByLastID",
    cursor_param: "after_id",
    parser_kind: "ParseAnthropicModels",
    spec_url: "https://github.com/anthropics/anthropic-sdk-typescript/blob/main/api.md",
    spec_format: "OpenAPI3",
};

static CEREBRAS_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static DEEPSEEK_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static FIREWORKS_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static GOOGLE_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1beta/models",
    pagination: "CursorOpaqueToken",
    cursor_param: "pageToken",
    parser_kind: "ParseGoogleModels",
    spec_url: "https://generativelanguage.googleapis.com/$discovery/rest?version=v1beta",
    spec_format: "GoogleDiscovery",
};

static GROK_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static GROQ_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static JAN_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static LLAMACPP_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static LMSTUDIO_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static MISTRAL_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "https://raw.githubusercontent.com/mistralai/platform-docs-public/main/openapi.yaml",
    spec_format: "OpenAPI3",
};

static MOONSHOT_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static OLLAMA_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "https://raw.githubusercontent.com/ollama/ollama/main/docs/openapi.yaml",
    spec_format: "OpenAPI3",
};

static OPENAI_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "https://github.com/openai/openai-openapi/blob/master/openapi.yaml",
    spec_format: "OpenAPI3",
};

static OPENROUTER_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "https://openrouter.ai/openapi.json",
    spec_format: "OpenAPI3",
};

static QWEN_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

static TOGETHER_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "https://raw.githubusercontent.com/togethercomputer/openapi/main/openapi.yaml",
    spec_format: "OpenAPI3",
};

static VLLM_CATALOGUE: CatalogueConfig = CatalogueConfig {
    endpoint: "/v1/models",
    pagination: "PaginationNone",
    cursor_param: "",
    parser_kind: "ParseOpenAICohortModels",
    spec_url: "",
    spec_format: "",
};

pub(crate) fn catalogue_config(provider: ProviderName) -> Option<&'static CatalogueConfig> {
    match provider {
        ProviderName::Anthropic => Some(&ANTHROPIC_CATALOGUE),
        ProviderName::Cerebras => Some(&CEREBRAS_CATALOGUE),
        ProviderName::Deepseek => Some(&DEEPSEEK_CATALOGUE),
        ProviderName::Fireworks => Some(&FIREWORKS_CATALOGUE),
        ProviderName::Google => Some(&GOOGLE_CATALOGUE),
        ProviderName::Grok => Some(&GROK_CATALOGUE),
        ProviderName::Groq => Some(&GROQ_CATALOGUE),
        ProviderName::Jan => Some(&JAN_CATALOGUE),
        ProviderName::Llamacpp => Some(&LLAMACPP_CATALOGUE),
        ProviderName::Lmstudio => Some(&LMSTUDIO_CATALOGUE),
        ProviderName::Mistral => Some(&MISTRAL_CATALOGUE),
        ProviderName::Moonshot => Some(&MOONSHOT_CATALOGUE),
        ProviderName::Ollama => Some(&OLLAMA_CATALOGUE),
        ProviderName::OpenAI => Some(&OPENAI_CATALOGUE),
        ProviderName::Openrouter => Some(&OPENROUTER_CATALOGUE),
        ProviderName::Qwen => Some(&QWEN_CATALOGUE),
        ProviderName::Together => Some(&TOGETHER_CATALOGUE),
        ProviderName::Vllm => Some(&VLLM_CATALOGUE),
        _ => None,
    }
}
