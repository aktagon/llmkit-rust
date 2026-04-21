#[derive(Clone, Debug, Default)]
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
