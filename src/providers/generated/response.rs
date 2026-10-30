// Code generated — DO NOT EDIT.

use super::providers::ProviderName;

pub fn response_text_path(provider: ProviderName) -> &'static str {
    match provider {
        ProviderName::AI21 => "choices[0].message.content",
        ProviderName::Anthropic => "content[0].text",
        ProviderName::Assemblyai => "",
        ProviderName::Azure => "choices[0].message.content",
        ProviderName::Bedrock => "output.message.content[0].text",
        ProviderName::Cerebras => "choices[0].message.content",
        ProviderName::Cohere => "choices[0].message.content",
        ProviderName::Deepseek => "choices[0].message.content",
        ProviderName::Doubao => "choices[0].message.content",
        ProviderName::Ernie => "choices[0].message.content",
        ProviderName::Fireworks => "choices[0].message.content",
        ProviderName::Google => "candidates[0].content.parts[0].text",
        ProviderName::Grok => "choices[0].message.content",
        ProviderName::Groq => "choices[0].message.content",
        ProviderName::Inworld => "",
        ProviderName::Jan => "choices[0].message.content",
        ProviderName::Llamacpp => "choices[0].message.content",
        ProviderName::Lmstudio => "choices[0].message.content",
        ProviderName::Minimax => "choices[0].message.content",
        ProviderName::Mistral => "choices[0].message.content",
        ProviderName::Moonshot => "choices[0].message.content",
        ProviderName::Ollama => "choices[0].message.content",
        ProviderName::OpenAI => "choices[0].message.content",
        ProviderName::Openrouter => "choices[0].message.content",
        ProviderName::Perplexity => "choices[0].message.content",
        ProviderName::Pixverse => "",
        ProviderName::Qwen => "choices[0].message.content",
        ProviderName::Recraft => "",
        ProviderName::Sambanova => "choices[0].message.content",
        ProviderName::Together => "choices[0].message.content",
        ProviderName::Vertex => "",
        ProviderName::Vidu => "",
        ProviderName::Vllm => "choices[0].message.content",
        ProviderName::Workersai => "choices[0].message.content",
        ProviderName::Yi => "choices[0].message.content",
        ProviderName::Zhipu => "choices[0].message.content",
    }
}

/// Where the assistant's text sits in a block-ARRAY response, located by
/// discriminator rather than array position: a leading thinking block or
/// non-text part shifts text out from under a fixed path (BUG-053).
///
/// * `marker_path` empty — every element is a text block
/// * `marker_path` set, `marker_value` empty — element is text if the key is PRESENT
/// * both set — element is text if the key EQUALS the value
///
/// `marker_value` is also a WRITE instruction: `encode_response` stamps it
/// onto the block it writes, so an emitted body reads back through this table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseTextConfig {
    pub blocks_path: &'static str,
    pub marker_path: &'static str,
    pub marker_value: &'static str,
    pub value_path: &'static str,
}

/// Text-block selector for a chat wire shape, or `None` when the shape
/// carries text as a plain scalar — `None` SELECTS the `response_text_path`
/// reader above, it does not mean the shape has no text.
pub fn response_text_config(chat_wire_shape: &str) -> Option<ResponseTextConfig> {
    match chat_wire_shape {
        "ChatAnthropic" => Some(ResponseTextConfig {
            blocks_path: "content",
            marker_path: "type",
            marker_value: "text",
            value_path: "text",
        }),
        "ChatBedrock" => Some(ResponseTextConfig {
            blocks_path: "output.message.content",
            marker_path: "text",
            marker_value: "",
            value_path: "text",
        }),
        "ChatGoogle" => Some(ResponseTextConfig {
            blocks_path: "candidates[0].content.parts",
            marker_path: "text",
            marker_value: "",
            value_path: "text",
        }),
        _ => None,
    }
}

/// Where a chat wire shape carries the response id (ADR-099 CRF-013). Read
/// for a call on a shape other than the provider's default; the default
/// shape reaches `ProviderConfig::response_id_path`.
pub fn wire_shape_response_id_path(chat_wire_shape: &str) -> &'static str {
    match chat_wire_shape {
        "ChatAnthropic" => "id",
        "ChatBedrock" => "",
        "ChatGoogle" => "responseId",
        "ChatOpenAI" => "id",
        "ChatResponsesOpenAI" => "id",
        _ => "",
    }
}

/// Where a chat wire shape carries the reported model, read like
/// `wire_shape_response_id_path`.
pub fn wire_shape_response_model_path(chat_wire_shape: &str) -> &'static str {
    match chat_wire_shape {
        "ChatAnthropic" => "model",
        "ChatBedrock" => "",
        "ChatGoogle" => "modelVersion",
        "ChatOpenAI" => "model",
        "ChatResponsesOpenAI" => "model",
        _ => "",
    }
}

/// Why a chat response stopped, in one vocabulary shared by every provider
/// (ADR-099 CRF-001). `Response::finish_reason` holds one; the provider's own
/// string is in `Response::provider_finish_reason`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FinishReason {
    /// The provider withheld or cut the output because its content filter flagged it.
    ContentFilter,
    /// The response reached the output-token limit set in the request.
    Length,
    /// Any wire value with no entry in the table (CRF-011). The provider's string is kept verbatim.
    Other,
    /// The model reached a natural stop point or a caller-provided stop sequence.
    Stop,
    /// The model stopped to call one or more tools.
    ToolCalls,
}

impl FinishReason {
    /// The canonical string: the value's CanonicalFinishReason label.
    pub fn as_str(self) -> &'static str {
        match self {
            FinishReason::ContentFilter => "content_filter",
            FinishReason::Length => "length",
            FinishReason::Other => "other",
            FinishReason::Stop => "stop",
            FinishReason::ToolCalls => "tool_calls",
        }
    }
}

impl std::fmt::Display for FinishReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of a chat wire shape's finish-reason table (ADR-099 CRF-004): a
/// provider wire value and the canonical value it means. `detail_path` and
/// `detail_value` are set only when the wire value fuses two canonical values;
/// the entry then matches only when the field at `detail_path` holds
/// `detail_value` (CRF-007). `preferred` marks the one entry per canonical
/// value that encoding writes (CRF-005).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinishReasonEntry {
    pub wire_value: &'static str,
    pub detail_path: &'static str,
    pub detail_value: &'static str,
    pub canonical: FinishReason,
    pub preferred: bool,
}

/// The finish-reason table of a chat wire shape, empty when the shape declares
/// none. A wire value with no entry is `FinishReason::Other` (CRF-011).
pub fn wire_shape_finish_reasons(chat_wire_shape: &str) -> &'static [FinishReasonEntry] {
    match chat_wire_shape {
        "ChatAnthropic" => &[
            FinishReasonEntry {
                wire_value: "end_turn",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Stop,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "max_tokens",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Length,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "refusal",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ContentFilter,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "stop_sequence",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Stop,
                preferred: false,
            },
            FinishReasonEntry {
                wire_value: "tool_use",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ToolCalls,
                preferred: true,
            },
        ],
        "ChatBedrock" => &[
            FinishReasonEntry {
                wire_value: "end_turn",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Stop,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "max_tokens",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Length,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "stop_sequence",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Stop,
                preferred: false,
            },
            FinishReasonEntry {
                wire_value: "tool_use",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ToolCalls,
                preferred: true,
            },
        ],
        "ChatGoogle" => &[
            FinishReasonEntry {
                wire_value: "BLOCKLIST",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ContentFilter,
                preferred: false,
            },
            FinishReasonEntry {
                wire_value: "MAX_TOKENS",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Length,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "PROHIBITED_CONTENT",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ContentFilter,
                preferred: false,
            },
            FinishReasonEntry {
                wire_value: "RECITATION",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ContentFilter,
                preferred: false,
            },
            FinishReasonEntry {
                wire_value: "SAFETY",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ContentFilter,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "SPII",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ContentFilter,
                preferred: false,
            },
            FinishReasonEntry {
                wire_value: "STOP",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Stop,
                preferred: true,
            },
        ],
        "ChatOpenAI" => &[
            FinishReasonEntry {
                wire_value: "content_filter",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ContentFilter,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "function_call",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ToolCalls,
                preferred: false,
            },
            FinishReasonEntry {
                wire_value: "length",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Length,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "stop",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Stop,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "tool_calls",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::ToolCalls,
                preferred: true,
            },
        ],
        "ChatResponsesOpenAI" => &[
            FinishReasonEntry {
                wire_value: "completed",
                detail_path: "",
                detail_value: "",
                canonical: FinishReason::Stop,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "incomplete",
                detail_path: "incomplete_details.reason",
                detail_value: "content_filter",
                canonical: FinishReason::ContentFilter,
                preferred: true,
            },
            FinishReasonEntry {
                wire_value: "incomplete",
                detail_path: "incomplete_details.reason",
                detail_value: "max_output_tokens",
                canonical: FinishReason::Length,
                preferred: true,
            },
        ],
        _ => &[],
    }
}

pub fn usage_paths(provider: ProviderName) -> (&'static str, &'static str) {
    match provider {
        ProviderName::AI21 => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Anthropic => ("usage.input_tokens", "usage.output_tokens"),
        ProviderName::Assemblyai => ("", ""),
        ProviderName::Azure => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Bedrock => ("usage.inputTokens", "usage.outputTokens"),
        ProviderName::Cerebras => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Cohere => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Deepseek => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Doubao => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Ernie => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Fireworks => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Google => (
            "usageMetadata.promptTokenCount",
            "usageMetadata.candidatesTokenCount",
        ),
        ProviderName::Grok => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Groq => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Inworld => ("", ""),
        ProviderName::Jan => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Llamacpp => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Lmstudio => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Minimax => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Mistral => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Moonshot => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Ollama => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::OpenAI => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Openrouter => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Perplexity => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Pixverse => ("", ""),
        ProviderName::Qwen => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Recraft => ("", ""),
        ProviderName::Sambanova => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Together => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Vertex => ("", ""),
        ProviderName::Vidu => ("", ""),
        ProviderName::Vllm => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Workersai => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Yi => ("usage.prompt_tokens", "usage.completion_tokens"),
        ProviderName::Zhipu => ("usage.prompt_tokens", "usage.completion_tokens"),
    }
}

pub fn usage_cost_path(provider: ProviderName) -> &'static str {
    match provider {
        ProviderName::AI21 => "",
        ProviderName::Anthropic => "",
        ProviderName::Assemblyai => "",
        ProviderName::Azure => "",
        ProviderName::Bedrock => "",
        ProviderName::Cerebras => "",
        ProviderName::Cohere => "",
        ProviderName::Deepseek => "",
        ProviderName::Doubao => "",
        ProviderName::Ernie => "",
        ProviderName::Fireworks => "",
        ProviderName::Google => "",
        ProviderName::Grok => "usage.cost_in_usd_ticks",
        ProviderName::Groq => "",
        ProviderName::Inworld => "",
        ProviderName::Jan => "",
        ProviderName::Llamacpp => "",
        ProviderName::Lmstudio => "",
        ProviderName::Minimax => "",
        ProviderName::Mistral => "",
        ProviderName::Moonshot => "",
        ProviderName::Ollama => "",
        ProviderName::OpenAI => "",
        ProviderName::Openrouter => "usage.cost",
        ProviderName::Perplexity => "",
        ProviderName::Pixverse => "",
        ProviderName::Qwen => "",
        ProviderName::Recraft => "",
        ProviderName::Sambanova => "",
        ProviderName::Together => "",
        ProviderName::Vertex => "",
        ProviderName::Vidu => "",
        ProviderName::Vllm => "",
        ProviderName::Workersai => "",
        ProviderName::Yi => "",
        ProviderName::Zhipu => "",
    }
}

pub fn usage_cost_scale(provider: ProviderName) -> f64 {
    match provider {
        ProviderName::Grok => 1e-10,
        _ => 1.0,
    }
}
