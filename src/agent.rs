use serde_json::{Map, Value};

use crate::error::Error;
use crate::middleware::{fire_post, fire_pre, Event, MiddlewareFn, MiddlewareOp};
use crate::options::PromptOptions;
use crate::providers::generated::providers::provider_config;
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::request::build_url;
use crate::response::{parse_api_error, parse_response};
use crate::structs::{ToolCall, ToolResult};
use crate::transforms::{extract_tool_calls, Msg};
use crate::{Provider, Request, Response, Tool, Usage};

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

    /// public_messages projects the agent's internal conversation
    /// history into the public Message shape for the typed builder's
    /// `bot.messages()` reader (ADR-020 HIST-004). Returns owned
    /// values; the internal `tool_result` role is flattened to `tool`
    /// to match the ontology's union-by-role discriminator. Each
    /// returned Message.tool_calls is a fresh Vec; the inner ToolCall
    /// instances are clones (Rust ownership precludes aliasing the
    /// agent's internal state through a borrow here without lifetime
    /// gymnastics that don't pay off for a small list).
    pub fn public_messages(&self) -> Vec<crate::structs::Message> {
        self.history
            .iter()
            .map(|m| {
                let role = if m.role == "tool_result" {
                    "tool".to_string()
                } else {
                    m.role.clone()
                };
                crate::structs::Message {
                    role,
                    content: m.content.clone(),
                    tool_calls: m.tool_calls.clone(),
                    tool_result: m.tool_result.clone(),
                }
            })
            .collect()
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
    /// ADR-020 HIST-007: seed the agent's internal history from a
    /// public Message list. Mechanical field copy; the public `tool`
    /// role is mapped back to the internal `tool_result`
    /// discriminator.
    pub fn seed_history(&mut self, messages: Vec<crate::structs::Message>) {
        self.history.clear();
        for m in messages {
            let role = if m.role == "tool" {
                "tool_result".to_string()
            } else {
                m.role
            };
            self.history.push(InternalMessage {
                role,
                content: m.content,
                tool_calls: m.tool_calls,
                tool_result: m.tool_result,
            });
        }
    }

    pub fn set_middleware(&mut self, middleware: Vec<MiddlewareFn>) {
        self.middleware = middleware;
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
        let model = crate::request::resolve_model(&self.provider, config)?;
        let url = build_url(&self.provider, config);
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

            // Build the request through the shared builder (ADR-026
            // PIPE-001/004): the agent constructs no body of its own. Its
            // trusted history is converted straight into the internal message
            // sum (PIPE-007) — no round-trip through the lossy public Message
            // shape — so the tool-aware message transforms and the
            // option/caching/structured-output steps all run identically to the
            // Text/batch path.
            let request = Request {
                system: self.system.clone(),
                ..Request::default()
            };
            let msgs = self.history_to_msgs();
            let (mut body, headers) = crate::request::build_request(
                &self.provider,
                &request,
                &msgs,
                &self.options,
                &self.tools,
            )?;
            // Caching is a shared request-construction step (ADR-026): applied
            // on every send path by construction, like the Text path. Before
            // this, a .caching() agent silently paid full input price every turn
            // (BUG-004). apply_caching no-ops when caching is off.
            crate::caching::apply_caching(&mut body, &self.provider, &self.options, config).await?;
            let llm_outcome: Result<(Value, Response), Error> = (async {
                let (status, response_body) =
                    if matches!(auth_scheme(self.provider.name), AuthScheme::SigV4) {
                        // ADR-052: caller custom headers ride alongside the
                        // signed Bedrock request (added post-signing).
                        let caller_headers: Vec<(String, String)> = self
                            .provider
                            .headers
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
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
                            body,
                            &self.provider.api_key,
                            &secret_key,
                            &session_token,
                            &region,
                            config.service_name,
                            &caller_headers,
                        )
                        .await?
                    } else {
                        crate::http::post_json(&url, body, &headers).await?
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
                        cost: resp.usage.cost,
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
            total_usage.cost += parsed_response.usage.cost;

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
                // ADR-020 widened ToolCall.input from Map<String, Value> to
                // Option<serde_json::Value>. Tool authors' run() callback still
                // takes a Map, so project the public field back into a Map
                // (defaulting to {} when input is null or non-object).
                let call_input_map: Map<String, Value> = call
                    .input
                    .as_ref()
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                let tool_event = Event {
                    op: MiddlewareOp::ToolCall,
                    provider: format!("{:?}", self.provider.name),
                    model: model.clone(),
                    tool: call.name.clone(),
                    args: call_input_map
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    ..Event::default()
                };
                let tool_start = std::time::Instant::now();
                fire_pre(&self.middleware, &tool_event)?;

                let content = match self.find_tool(&call.name) {
                    Some(tool) => tool
                        .run(call_input_map)
                        .unwrap_or_else(|error| format!("error: {error}")),
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

    /// Converts the agent's trusted internal history directly into the internal
    /// message sum (ADR-026 PIPE-007), bypassing the public Message shape. The
    /// agent sets exactly one carrier per turn by construction, so the
    /// to_internal carrier check is unnecessary here — that boundary guards only
    /// untrusted, user-supplied Message lists on the Text/batch path.
    fn history_to_msgs(&self) -> Vec<Msg> {
        self.history
            .iter()
            .map(|m| {
                if let Some(result) = &m.tool_result {
                    Msg::Result(result.clone())
                } else if !m.tool_calls.is_empty() {
                    Msg::Calls(m.tool_calls.clone())
                } else {
                    Msg::Text {
                        role: m.role.clone(),
                        text: m.content.clone(),
                    }
                }
            })
            .collect()
    }

    fn find_tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}
