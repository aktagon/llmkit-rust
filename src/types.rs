use crate::ProviderName;

#[derive(Clone, Debug)]
pub struct Provider {
    pub name: ProviderName,
    pub api_key: String,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

impl Provider {
    pub fn new(name: ProviderName, api_key: impl Into<String>) -> Self {
        Self {
            name,
            api_key: api_key.into(),
            model: None,
            base_url: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Request {
    pub system: Option<String>,
    pub user: Option<String>,
    pub messages: Vec<Message>,
    pub schema: Option<String>,
    pub files: Vec<File>,
    pub images: Vec<InputImage>,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct File {
    pub id: String,
    pub uri: String,
    pub mime_type: String,
    pub name: String,
}

/// Image attached to a text-generation request (vision input).
///
/// Distinct from `Part::Image(MediaRef)` used for image-generation calls.
/// The two concepts target different capabilities; aligning text generation
/// onto Part-based vocabulary is tracked separately (ADR-008 OQ-2).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InputImage {
    pub url: String,
    pub mime_type: String,
    pub detail: String,
}

#[derive(Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
    pub run: std::sync::Arc<
        dyn Fn(serde_json::Map<String, serde_json::Value>) -> Result<String, String> + Send + Sync,
    >,
}

impl Tool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: serde_json::Value,
        run: impl Fn(serde_json::Map<String, serde_json::Value>) -> Result<String, String>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            schema,
            run: std::sync::Arc::new(run),
        }
    }

    pub fn run(&self, args: serde_json::Map<String, serde_json::Value>) -> Result<String, String> {
        (self.run)(args)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
    pub cache_write: u32,
    pub cache_read: u32,
    pub reasoning: u32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SafetySetting {
    pub category: String,
    pub threshold: String,
}

// Harm category constants for SafetySetting.category
pub const HARM_CATEGORY_HARASSMENT: &str = "HARM_CATEGORY_HARASSMENT";
pub const HARM_CATEGORY_HATE_SPEECH: &str = "HARM_CATEGORY_HATE_SPEECH";
pub const HARM_CATEGORY_SEXUALLY_EXPLICIT: &str = "HARM_CATEGORY_SEXUALLY_EXPLICIT";
pub const HARM_CATEGORY_DANGEROUS_CONTENT: &str = "HARM_CATEGORY_DANGEROUS_CONTENT";
pub const HARM_CATEGORY_CIVIC_INTEGRITY: &str = "HARM_CATEGORY_CIVIC_INTEGRITY";

// Harm block threshold constants for SafetySetting.threshold
pub const HARM_BLOCK_THRESHOLD_NONE: &str = "BLOCK_NONE";
pub const HARM_BLOCK_THRESHOLD_LOW_AND_ABOVE: &str = "BLOCK_LOW_AND_ABOVE";
pub const HARM_BLOCK_THRESHOLD_MEDIUM_AND_ABOVE: &str = "BLOCK_MEDIUM_AND_ABOVE";
pub const HARM_BLOCK_THRESHOLD_HIGH_ONLY: &str = "BLOCK_ONLY_HIGH";

// Vertex Imagen safety filter threshold constants
pub const IMAGE_SAFETY_FILTER_BLOCK_FEW: &str = "block_few";
pub const IMAGE_SAFETY_FILTER_BLOCK_SOME: &str = "block_some";
pub const IMAGE_SAFETY_FILTER_BLOCK_MOST: &str = "block_most";
pub const IMAGE_SAFETY_FILTER_BLOCK_ONLY_HIGH: &str = "block_only_high";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    pub text: String,
    pub usage: Usage,
    /// Provider stop signal, passed through verbatim. Empty when the
    /// provider response carries no signal or the parser does not yet
    /// read this provider's location. Examples per provider:
    ///   Google:    "STOP", "MAX_TOKENS", "SAFETY", "RECITATION"
    ///   OpenAI:    "stop", "length", "content_filter", "tool_calls"
    ///   Anthropic: "end_turn", "max_tokens", "stop_sequence", "tool_use"
    ///   xAI:       "stop", "length", "content_filter"
    pub finish_reason: String,
    /// Free-text provider explanation of the stop signal. Populated by
    /// Google when present; OpenAI / Anthropic / xAI do not carry an
    /// equivalent field, so this stays empty for them.
    pub finish_message: String,
    /// Parsed provider response body, populated only when the caller
    /// opted in via the builder's `.raw()` chain method (ADR-014).
    /// Type-erased — consumers walk `serde_json::Value` or deserialize
    /// into a provider-shape struct of their choice.
    pub raw: Option<serde_json::Value>,
}
