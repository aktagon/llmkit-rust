use serde_json::{json, Map, Value};

use crate::error::Error;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::options::PromptOptions;
use crate::providers::generated::options::OptionKey;
use crate::providers::generated::providers::{provider_config, ProviderConfig};
use crate::providers::generated::request::{auth_scheme, system_placement, AuthScheme, SystemPlacement};
use crate::request::{build_auth_headers, build_url};
use crate::response::{parse_api_error, parse_response};
use crate::transforms::{apply_tool_defs, extract_tool_calls, tool_call_message, tool_result_message, ToolCall, ToolResult};
use crate::{supported_options, Provider, Request, Response, Tool, Usage};

#[derive(Clone, Debug)]
struct InternalMessage {
    role: String,
    content: String,
    tool_calls: Vec<ToolCall>,
    tool_result: Option<ToolResult>,
}

pub struct Agent {
    provider: Provider,
    options: PromptOptions,
    tools: Vec<Tool>,
    history: Vec<InternalMessage>,
    system: Option<String>,
    max_tool_iterations: usize,
    middleware: Vec<MiddlewareFn>,
}

impl Agent {
    pub fn new(provider: Provider) -> Self {
        Self {
            provider,
            options: PromptOptions::new(),
            tools: Vec::new(),
            history: Vec::new(),
            system: None,
            max_tool_iterations: 10,
            middleware: Vec::new(),
        }
    }

    pub fn set_system(&mut self, system: impl Into<String>) {
        self.system = Some(system.into());
    }

    pub fn set_options(&mut self, options: PromptOptions) {
        self.options = options;
    }

    pub fn set_max_tool_iterations(&mut self, iterations: usize) {
        self.max_tool_iterations = iterations;
    }

    pub fn add_tool(&mut self, tool: Tool) {
        self.tools.push(tool);
    }

    /// Register one or more middleware hooks. Each hook fires around every
    /// LLM call (`MiddlewareOp::LlmRequest`) and every tool invocation
    /// (`MiddlewareOp::ToolCall`) the agent performs.
    pub fn with_middleware(mut self, middleware: Vec<MiddlewareFn>) -> Self {
        self.middleware = middleware;
        self
    }


    pub async fn chat(&mut self, message: impl Into<String>) -> Result<Response, Error> {
        self.history.push(InternalMessage {
            role: "user".into(),
            content: message.into(),
            tool_calls: Vec::new(),
            tool_result: None,
        });
        self.run_tool_loop().await
    }

    async fn run_tool_loop(&mut self) -> Result<Response, Error> {
        crate::request::validate_provider(&self.provider)?;
        let config = provider_config(self.provider.name);
        let url = build_url(&self.provider, config);
        let model = self
            .provider
            .model
            .clone()
            .unwrap_or_else(|| config.default_model.to_string());
        let mut total_usage = Usage::default();

        for _ in 0..self.max_tool_iterations {
            // Fire LlmRequest middleware around each turn of the agent loop.
            let llm_event = Event {
                op: MiddlewareOp::LlmRequest,
                provider: format!("{:?}", self.provider.name),
                model: model.clone(),
                ..Event::default()
            };
            let llm_start = std::time::Instant::now();
            fire_pre(&self.middleware, &llm_event)?;

            let (body, headers) = self.build_request(config)?;
            let llm_outcome: Result<(Value, Response), Error> = (async {
                let (status, response_body) =
                    if matches!(auth_scheme(self.provider.name), AuthScheme::SigV4) {
                        let region = std::env::var(config.region_env_var).map_err(|_| Error::Validation {
                            field: "provider",
                            message: format!("missing env var {}", config.region_env_var),
                        })?;
                        let secret_key =
                            std::env::var(config.secret_key_env_var).map_err(|_| Error::Validation {
                                field: "provider",
                                message: format!("missing env var {}", config.secret_key_env_var),
                            })?;
                        let session_token = if config.session_token_env_var.is_empty() {
                            String::new()
                        } else {
                            std::env::var(config.session_token_env_var).unwrap_or_default()
                        };
                        crate::http::post_json_sigv4(
                            &url,
                            Value::Object(body),
                            &self.provider.api_key,
                            &secret_key,
                            &session_token,
                            &region,
                            config.service_name,
                        )
                        .await?
                    } else {
                        crate::http::post_json(&url, Value::Object(body), &headers).await?
                    };
                if !status.is_success() {
                    return Err(parse_api_error(&self.provider, status.as_u16(), &response_body));
                }
                let parsed: Value = serde_json::from_str(&response_body)?;
                let parsed_response = parse_response(&self.provider, &response_body)?;
                Ok((parsed, parsed_response))
            })
            .await;

            let mut llm_post = llm_event.clone();
            llm_post.duration = Some(llm_start.elapsed());
            match &llm_outcome {
                Ok((_, resp)) => {
                    llm_post.usage = Some(crate::middleware::Usage {
                        input: resp.usage.input as i64,
                        output: resp.usage.output as i64,
                        cache_write: resp.usage.cache_write as i64,
                        cache_read: resp.usage.cache_read as i64,
                        reasoning: resp.usage.reasoning as i64,
                    })
                }
                Err(err) => llm_post.err = Some(err.to_string()),
            }
            fire_post(&self.middleware, &llm_post);

            let (parsed, parsed_response) = llm_outcome?;
            total_usage.input += parsed_response.usage.input;
            total_usage.output += parsed_response.usage.output;
            total_usage.cache_write += parsed_response.usage.cache_write;
            total_usage.cache_read += parsed_response.usage.cache_read;
            total_usage.reasoning += parsed_response.usage.reasoning;

            let calls = extract_tool_calls(&parsed, config);
            if calls.is_empty() {
                self.history.push(InternalMessage {
                    role: "assistant".into(),
                    content: parsed_response.text.clone(),
                    tool_calls: Vec::new(),
                    tool_result: None,
                });
                return Ok(Response {
                    text: parsed_response.text,
                    usage: total_usage,
                    finish_reason: parsed_response.finish_reason,
                    finish_message: parsed_response.finish_message,
                    raw: if self.options.raw { Some(parsed) } else { None },
                });
            }

            self.history.push(InternalMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: calls.clone(),
                tool_result: None,
            });

            for call in calls {
                // Fire ToolCall middleware around each tool invocation.
                let tool_event = Event {
                    op: MiddlewareOp::ToolCall,
                    provider: format!("{:?}", self.provider.name),
                    model: model.clone(),
                    tool: call.name.clone(),
                    args: call
                        .input
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    ..Event::default()
                };
                let tool_start = std::time::Instant::now();
                fire_pre(&self.middleware, &tool_event)?;

                let content = match self.find_tool(&call.name) {
                    Some(tool) => tool.run(call.input.clone()).unwrap_or_else(|error| format!("error: {error}")),
                    None => format!("error: unknown tool {:?}", call.name),
                };

                let mut tool_post = tool_event.clone();
                tool_post.result = content.clone();
                tool_post.duration = Some(tool_start.elapsed());
                fire_post(&self.middleware, &tool_post);

                self.history.push(InternalMessage {
                    role: "tool_result".into(),
                    content: String::new(),
                    tool_calls: Vec::new(),
                    tool_result: Some(ToolResult {
                        tool_use_id: call.id,
                        content,
                    }),
                });
            }
        }

        Err(Error::Unsupported(format!(
            "max tool iterations ({}) reached",
            self.max_tool_iterations
        )))
    }

    fn build_request(&self, config: &ProviderConfig) -> Result<(Map<String, Value>, Vec<(String, String)>), Error> {
        let model = self
            .provider
            .model
            .clone()
            .unwrap_or_else(|| config.default_model.to_string());

        let mut body = Map::new();
        let headers = build_auth_headers(&self.provider, config);

        if config.model_in_body {
            body.insert("model".into(), Value::String(model));
        }

        if let Some(key) = json_key_for(self.provider.name, OptionKey::MaxTokens) {
            let max_tokens = self.options.max_tokens.unwrap_or(config.default_max_tokens);
            insert_nested_field(&mut body, key, json!(max_tokens));
        }

        match system_placement(self.provider.name) {
            SystemPlacement::TopLevelField => {
                if let Some(system) = &self.system {
                    if config.wraps_options_in == "inferenceConfig" && config.auth_scheme == "SigV4" {
                        body.insert("system".into(), json!([{ "text": system }]));
                    } else {
                        body.insert("system".into(), Value::String(system.clone()));
                    }
                }
            }
            SystemPlacement::MessageInArray => {}
            SystemPlacement::SiblingObject => {
                if let Some(system) = &self.system {
                    body.insert(
                        "system_instruction".into(),
                        json!({"parts": [{"text": system}]}),
                    );
                }
            }
        }

        self.apply_history(&mut body, config);
        apply_tool_defs(&mut body, config, &self.tools);
        add_options(&mut body, &self.provider, &self.options, config.default_max_tokens);

        Ok((body, headers))
    }

    fn apply_history(&self, body: &mut Map<String, Value>, config: &ProviderConfig) {
        let has_tool_messages = self
            .history
            .iter()
            .any(|message| message.tool_result.is_some() || !message.tool_calls.is_empty());

        if !has_tool_messages {
            let request = Request {
                system: self.system.clone(),
                user: None,
                messages: self
                    .history
                    .iter()
                    .map(|message| crate::Message::new(message.role.clone(), message.content.clone()))
                    .collect(),
                schema: None,
                files: Vec::new(),
                images: Vec::new(),
            };
            apply_basic_message_shape(body, &request, config);
            return;
        }

        if matches!(system_placement(config.name), SystemPlacement::SiblingObject) {
            let contents = self
                .history
                .iter()
                .map(|message| {
                    if let Some(result) = &message.tool_result {
                        tool_result_message(config, result)
                    } else if !message.tool_calls.is_empty() {
                        tool_call_message(config, &message.tool_calls)
                    } else {
                        json!({
                            "role": map_role(&message.role, config),
                            "parts": [{"text": message.content}],
                        })
                    }
                })
                .collect();
            body.insert("contents".into(), Value::Array(contents));
        } else {
            let mut messages = Vec::new();
            if matches!(system_placement(config.name), SystemPlacement::MessageInArray) {
                if let Some(system) = &self.system {
                    messages.push(json!({
                        "role": map_role("system", config),
                        "content": system,
                    }));
                }
            }
            for message in &self.history {
                if let Some(result) = &message.tool_result {
                    messages.push(tool_result_message(config, result));
                } else if !message.tool_calls.is_empty() {
                    messages.push(tool_call_message(config, &message.tool_calls));
                } else {
                    messages.push(json!({
                        "role": map_role(&message.role, config),
                        "content": message.content,
                    }));
                }
            }
            body.insert("messages".into(), Value::Array(messages));
        }
    }

    fn find_tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}

fn apply_basic_message_shape(body: &mut Map<String, Value>, request: &Request, config: &ProviderConfig) {
    match system_placement(config.name) {
        SystemPlacement::SiblingObject => {
            let contents = request
                .messages
                .iter()
                .map(|message| {
                    json!({
                        "role": map_role(&message.role, config),
                        "parts": [{"text": message.content}],
                    })
                })
                .collect();
            body.insert("contents".into(), Value::Array(contents));
        }
        SystemPlacement::TopLevelField | SystemPlacement::MessageInArray => {
            let mut messages = Vec::new();
            if matches!(system_placement(config.name), SystemPlacement::MessageInArray) {
                if let Some(system) = &request.system {
                    messages.push(json!({
                        "role": map_role("system", config),
                        "content": system,
                    }));
                }
            }
            messages.extend(request.messages.iter().map(|message| {
                json!({
                    "role": map_role(&message.role, config),
                    "content": message.content,
                })
            }));
            body.insert("messages".into(), Value::Array(messages));
        }
    }
}

fn add_options(body: &mut Map<String, Value>, provider: &Provider, options: &PromptOptions, default_max_tokens: u32) {
    let config = provider_config(provider.name);
    if config.wraps_options_in.is_empty() {
        add_options_into(body, provider, options, default_max_tokens);
        return;
    }

    let mut wrapped = Map::new();
    add_options_into(&mut wrapped, provider, options, default_max_tokens);
    if !wrapped.is_empty() {
        body.insert(config.wraps_options_in.into(), Value::Object(wrapped));
    }
}

fn add_options_into(body: &mut Map<String, Value>, provider: &Provider, options: &PromptOptions, default_max_tokens: u32) {
    maybe_insert(body, provider, OptionKey::Temperature, options.temperature.map(Value::from));
    maybe_insert(body, provider, OptionKey::TopP, options.top_p.map(Value::from));
    maybe_insert(body, provider, OptionKey::TopK, options.top_k.map(Value::from));
    maybe_insert(body, provider, OptionKey::Seed, options.seed.map(Value::from));
    maybe_insert(
        body,
        provider,
        OptionKey::FrequencyPenalty,
        options.frequency_penalty.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        OptionKey::PresencePenalty,
        options.presence_penalty.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        OptionKey::ThinkingBudget,
        options.thinking_budget.map(Value::from),
    );
    maybe_insert(
        body,
        provider,
        OptionKey::ReasoningEffort,
        options.reasoning_effort.clone().map(Value::from),
    );
    if !options.stop_sequences.is_empty() {
        maybe_insert(
            body,
            provider,
            OptionKey::StopSequences,
            Some(Value::Array(
                options
                    .stop_sequences
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            )),
        );
    }

    if let Some(key) = json_key_for(provider.name, OptionKey::MaxTokens) {
        insert_nested_field(
            body,
            key,
            Value::from(options.max_tokens.unwrap_or(default_max_tokens)),
        );
    }
}

fn maybe_insert(body: &mut Map<String, Value>, provider: &Provider, key: OptionKey, value: Option<Value>) {
    let Some(value) = value else {
        return;
    };
    if let Some(json_key) = json_key_for(provider.name, key) {
        insert_nested_field(body, json_key, value);
    }
}

fn json_key_for(provider: crate::ProviderName, key: OptionKey) -> Option<&'static str> {
    supported_options(provider)
        .iter()
        .find(|option| option.key == key)
        .map(|option| option.json_key)
}

fn insert_nested_field(body: &mut Map<String, Value>, path: &str, value: Value) {
    let mut current = body;
    let mut parts = path.split('.').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.insert(part.to_string(), value);
            return;
        }
        let entry = current
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current = entry.as_object_mut().expect("nested option object");
    }
}

fn map_role(role: &str, config: &ProviderConfig) -> String {
    config
        .role_mappings
        .iter()
        .find(|(from, _)| *from == role)
        .map(|(_, to)| (*to).to_string())
        .unwrap_or_else(|| role.to_string())
}
