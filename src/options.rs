use crate::middleware::MiddlewareFn;
use crate::types::SafetySetting;

#
pub struct PromptOptions {
    pub caching: bool,
    pub cache_ttl: Option<u32>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<u32>,
    pub max_tokens: Option<u32>,
    pub stop_sequences: Vec<String>,
    pub seed: Option<i64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<String>,
    ///
    ///
    pub protocol: Option<String>,
    pub middleware: Vec<MiddlewareFn>,
    pub safety_settings: Vec<SafetySetting>,
    ///
    ///
    ///
    pub raw: bool,
}

impl std::fmt::Debug for PromptOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptOptions")
            .field("caching", &self.caching)
            .field("cache_ttl", &self.cache_ttl)
            .field("temperature", &self.temperature)
            .field("top_p", &self.top_p)
            .field("top_k", &self.top_k)
            .field("max_tokens", &self.max_tokens)
            .field("stop_sequences", &self.stop_sequences)
            .field("seed", &self.seed)
            .field("frequency_penalty", &self.frequency_penalty)
            .field("presence_penalty", &self.presence_penalty)
            .field("thinking_budget", &self.thinking_budget)
            .field("reasoning_effort", &self.reasoning_effort)
            .field("protocol", &self.protocol)
            .field("middleware", &format!("[{} fns]", self.middleware.len()))
            .field(
                "safety_settings",
                &format!("[{} settings]", self.safety_settings.len()),
            )
            .field("raw", &self.raw)
            .finish()
    }
}

impl PromptOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn temperature(mut self, value: f64) -> Self {
        self.temperature = Some(value);
        self
    }

    pub fn caching(mut self) -> Self {
        self.caching = true;
        self
    }

    pub fn cache_ttl(mut self, seconds: u32) -> Self {
        self.cache_ttl = Some(seconds);
        self
    }

    pub fn top_p(mut self, value: f64) -> Self {
        self.top_p = Some(value);
        self
    }

    pub fn top_k(mut self, value: u32) -> Self {
        self.top_k = Some(value);
        self
    }

    pub fn max_tokens(mut self, value: u32) -> Self {
        self.max_tokens = Some(value);
        self
    }

    pub fn stop_sequences<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.stop_sequences = values.into_iter().map(Into::into).collect();
        self
    }

    pub fn seed(mut self, value: i64) -> Self {
        self.seed = Some(value);
        self
    }

    pub fn frequency_penalty(mut self, value: f64) -> Self {
        self.frequency_penalty = Some(value);
        self
    }

    pub fn presence_penalty(mut self, value: f64) -> Self {
        self.presence_penalty = Some(value);
        self
    }

    pub fn thinking_budget(mut self, value: u32) -> Self {
        self.thinking_budget = Some(value);
        self
    }

    pub fn reasoning_effort(mut self, value: impl Into<String>) -> Self {
        self.reasoning_effort = Some(value.into());
        self
    }
}
