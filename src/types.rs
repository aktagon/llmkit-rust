use crate::structs::{File, Message};
use crate::ProviderName;

/// Build one with [`Provider::new`] and set fields on it; `#[non_exhaustive]`
/// keeps a future field from breaking callers again (BUG-062 added `timeout`).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Provider {
    pub name: ProviderName,
    pub api_key: String,
    pub model: Option<String>,
    pub base_url: Option<String>,
    /// Custom HTTP headers added via Client::add_header (ADR-052). Merged into
    /// every request before the provider auth header and the static required
    /// header, so a gateway header (e.g. cf-aig-authorization) rides alongside
    /// the provider key without clobbering it.
    pub headers: std::collections::HashMap<String, String>,
    /// How long a request waits for the next response bytes (BUG-062),
    /// copied from `ProviderConfig::timeout`. `Duration::ZERO` disables it.
    pub timeout: std::time::Duration,
}

/// Capability names one of the SDK's modelled capabilities. The set mirrors
///
///
/// provider wire data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Capability {
    ChatCompletion,
    ImageGeneration,
    ToolCalling,
    FileUpload,
    Batching,
    Caching,
    Reasoning,
    Catalogue,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::ChatCompletion => "chat_completion",
            Capability::ImageGeneration => "image_generation",
            Capability::ToolCalling => "tool_calling",
            Capability::FileUpload => "file_upload",
            Capability::Batching => "batching",
            Capability::Caching => "caching",
            Capability::Reasoning => "reasoning",
            Capability::Catalogue => "catalogue",
        }
    }
}

impl Provider {
    pub fn new(name: ProviderName, api_key: impl Into<String>) -> Self {
        Self {
            name,
            api_key: api_key.into(),
            model: None,
            base_url: None,
            headers: std::collections::HashMap::new(),
            timeout: crate::builders::DEFAULT_TIMEOUT,
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

impl crate::structs::Message {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            ..Default::default()
        }
    }
}

/// Image attached to a text-generation request (vision input).
///
/// The Text builder's `.image(mime, bytes)` part lowers into this carrier as a
/// base64 data URI and reaches the wire as the provider's native image block
/// (ADR-060). Distinct from `Part::Image(MediaRef)` used for image-generation
/// calls; unifying text-gen input onto Part vocabulary wholesale remains future
/// work.
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

// Usage is GENERATED from the TokenDimension instances (plus the ADR-027
// cost field) into providers/generated/middleware.rs, and re-exported here so
// the hand-written surface keeps one name for it. It used to be redeclared in
// this file, which meant two types named Usage in one crate: the generated one
// was i64 and gained optional dimensions (ADR-081) while this copy stayed u32
// and non-optional, and five hand-written `usage_to_event` converters carried
// values between them. A converter between a type and itself is the drift
// announcing itself.
pub use crate::providers::generated::middleware::Usage;

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
