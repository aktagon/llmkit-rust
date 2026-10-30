use serde_json::{Map, Value};

use crate::error::Error;
use crate::middleware::{fire_post, fire_pre, set_event_error, Event, MiddlewareOp};
use crate::options::SendOptions;
use crate::providers::generated::providers::{provider_config, ProviderSpec};
use crate::providers::generated::request::{auth_scheme, AuthScheme};
use crate::request::{encode_body, prepare_request, request_path, send_headers, send_url};
use crate::response::{accumulate_usage, decode_response_raw, parse_api_error};
use crate::structs::{Message, Part, ToolResult};
use crate::transforms::to_internal;
use crate::{Provider, Request, Response, Tool, Usage};

pub struct Agent {
    provider: Provider,
    /// The Request every turn sends, without its messages: the model, system,
    /// tool definitions and options (ADR-098 RQC-001). The history fills
    /// `messages` on each turn.
    request: Request,
    /// The functions that run the tools, kept beside the Request, which holds
    /// only their definitions (RQC-018).
    tools: Vec<Tool>,
    options: SendOptions,
    /// The conversation as public Messages (ADR-100): an assistant turn the
    /// loop received keeps the provider's own bytes in provider_turn (ADR-085)
    /// and is replayed verbatim on every later request in the run.
    history: Vec<Message>,
    max_tool_iterations: usize,
}

impl Agent {
    pub(crate) fn new(
        provider: Provider,
        request: Request,
        tools: Vec<Tool>,
        options: SendOptions,
    ) -> Self {
        Self {
            provider,
            request,
            tools,
            options,
            history: Vec::new(),
            max_tool_iterations: 10,
        }
    }

    /// public_messages returns a copy of the agent's conversation history
    /// for the typed builder's `bot.messages()` reader (ADR-020 HIST-004).
    /// The provider_turn of each turn crosses too (ADR-085), so save_history
    /// serializes the turn the next request would replay.
    pub fn public_messages(&self) -> Vec<Message> {
        self.history.clone()
    }

    pub fn set_max_tool_iterations(&mut self, iterations: usize) {
        self.max_tool_iterations = iterations;
    }

    /// ADR-020 HIST-007: seed the agent's history from a public Message
    /// list. A turn restored from load_history carries its provider_turn back
    /// into the loop, which is what makes cross-process resume (ADR-023)
    /// replay rather than reconstruct.
    pub fn seed_history(&mut self, messages: Vec<Message>) {
        self.history = messages;
    }

    /// The request is prepared (validated, model resolved) with the user turn
    /// before the turn joins the history, so a rejected request leaves the
    /// history as it was (HANDOFF-105 F3). Every turn of the loop sends the
    /// same Request with a longer history, so one preparation holds for all
    /// of them.
    ///
    /// Once the call starts it is all-or-nothing (owner decision 4,
    /// HANDOFF-105 F4). The call builds its turns in a list of its own and
    /// adds them to the history only when it succeeds (HANDOFF-105 F5), so a
    /// dropped `chat` future leaves the history as it was, and on any error
    /// `Error::Agent` carries the turns the call made, tool results included.
    pub async fn chat(&mut self, message: impl Into<String>) -> Result<Response, Error> {
        crate::request::validate_provider(&self.provider)?;
        let config = provider_config(self.provider.name);
        let mut messages = self.history.clone();
        // A user turn is llmkit's own, never a provider's.
        messages.push(Message::new("user", message));
        let mut prepared = prepare_request(
            &self.provider,
            config,
            &Request {
                messages,
                ..self.request.clone()
            },
        )?;
        let mut turns = prepared.messages.split_off(self.history.len());
        match self
            .run_tool_loop(config, prepared.clone(), &mut turns)
            .await
        {
            Ok(response) => {
                self.history = prepared.messages;
                self.history.append(&mut turns);
                Ok(response)
            }
            Err(source) => Err(Error::Agent {
                source: Box::new(source),
                unsaved_turns: turns,
            }),
        }
    }

    /// `request` is the prepared Request; its messages are the history, and
    /// each turn sends them with the call's turns so far, and appends each new
    /// turn to `turns`.
    async fn run_tool_loop(
        &self,
        config: &'static ProviderSpec,
        request: Request,
        turns: &mut Vec<Message>,
    ) -> Result<Response, Error> {
        // The middleware fires around every LLM call (`MiddlewareOp::LlmRequest`)
        // and every tool invocation (`MiddlewareOp::ToolCall`).
        let middleware = self.options.middleware.clone();
        // Seeded from the first turn, not from a default: absorbing addition's
        // identity is a REPORTED zero, and `Usage::default()` reports nothing,
        // so an all-None seed would absorb every turn back to nothing
        // (ADR-081 AVAIL-005).
        let mut total_usage: Option<Usage> = None;

        for _ in 0..self.max_tool_iterations {
            // Build the request through the shared setup step and encoder
            // (ADR-026 PIPE-001, ADR-098 RQC-001, RQC-017): the agent
            // constructs no body of its own. Its history becomes the Request's
            // messages and goes through the same to_internal boundary as the
            // Text/batch path (ADR-100); chat ran the shared setup, so
            // validation, model resolution, the tool-aware message transforms
            // and the option/caching/structured-output steps all run
            // identically to it.
            let turn_request = Request {
                messages: request
                    .messages
                    .iter()
                    .chain(turns.iter())
                    .cloned()
                    .collect(),
                ..request.clone()
            };
            let model = turn_request.model.clone();
            let url = send_url(&self.provider, config, &request_path(config, &model));

            // Fire LlmRequest middleware around each turn of the agent loop.
            let llm_event = Event {
                op: MiddlewareOp::LlmRequest,
                provider: format!("{:?}", self.provider.name),
                model: model.clone(),
                ..Event::default()
            };
            let llm_start = crate::clock::Instant::now();
            fire_pre(&middleware, &llm_event)?;

            let msgs = to_internal(&turn_request.messages)?;
            let (mut body, content) =
                encode_body(self.provider.name, config, &turn_request, &msgs)?;
            let headers = send_headers(&self.provider, config, &content);
            // Caching is a shared request-construction step (ADR-026): applied
            // on every send path by construction, like the Text path. Before
            // this, a .caching() agent silently paid full input price every turn
            // (BUG-004). apply_caching no-ops when caching is off.
            crate::caching::apply_caching(&mut body, &self.provider, &model, &self.options, config)
                .await?;
            let llm_outcome: Result<Response, Error> = (async {
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
                        let region = std::env::var(config.region_env_var).map_err(|_| {
                            Error::Validation {
                                field: "provider".into(),
                                message: format!("missing env var {}", config.region_env_var),
                            }
                        })?;
                        let secret_key =
                            std::env::var(config.secret_key_env_var).map_err(|_| {
                                Error::Validation {
                                    field: "provider".into(),
                                    message: format!(
                                        "missing env var {}",
                                        config.secret_key_env_var
                                    ),
                                }
                            })?;
                        let session_token = if config.session_token_env_var.is_empty() {
                            String::new()
                        } else {
                            std::env::var(config.session_token_env_var).unwrap_or_default()
                        };
                        crate::http::post_json_sigv4(
                            &url,
                            self.provider.timeout,
                            body,
                            &crate::http::SigV4Credentials {
                                access_key: &self.provider.api_key,
                                secret_key: &secret_key,
                                session_token: &session_token,
                                region: &region,
                                service: config.service_name,
                            },
                            &caller_headers,
                        )
                        .await?
                    } else {
                        crate::http::post_json(&url, self.provider.timeout, body, &headers).await?
                    };
                if !status.is_success() {
                    return Err(parse_api_error(
                        &self.provider,
                        status.as_u16(),
                        &response_body,
                    ));
                }
                decode_response_raw(
                    self.provider.name,
                    config.chat_wire_shape,
                    &response_body,
                    self.options.raw,
                )
            })
            .await;

            let mut llm_post = llm_event.clone();
            llm_post.duration = Some(llm_start.elapsed());
            match &llm_outcome {
                // Per-TURN usage, not the running total: a middleware observing
                // one request should see what that request consumed.
                Ok(resp) => llm_post.usage = Some(resp.usage),
                Err(err) => set_event_error(&mut llm_post, err),
            }
            fire_post(&middleware, &llm_post);

            let parsed_response = llm_outcome?;
            total_usage = Some(match total_usage {
                Some(total) => accumulate_usage(total, parsed_response.usage),
                None => parsed_response.usage,
            });

            // Record the assistant turn as the response's parts: its text and
            // its tool calls, in the order the provider sent them (owner
            // decision 1, HANDOFF-105 F4). provider_turn is the same turn as
            // the provider wrote it, and is what the NEXT request sends
            // (ADR-085). Before F4 the loop kept only the tool calls of a tool
            // turn, which dropped the model's text beside them.
            turns.push(Message {
                role: "assistant".into(),
                parts: parsed_response.parts.clone(),
                provider_turn: parsed_response.provider_turn.clone(),
            });
            let calls = parsed_response.tool_calls();
            if calls.is_empty() {
                // The terminal Response is the decoder's, with the loop's
                // usage total (ADR-076 SYM-004: one reader).
                return Ok(Response {
                    usage: total_usage.unwrap_or_default(),
                    ..parsed_response
                });
            }

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
                let tool_start = crate::clock::Instant::now();
                fire_pre(&middleware, &tool_event)?;

                let content = match self.find_tool(&call.name) {
                    Some(tool) => tool
                        .run(call_input_map)
                        .unwrap_or_else(|error| format!("error: {error}")),
                    None => format!("error: unknown tool {:?}", call.name),
                };

                let mut tool_post = tool_event.clone();
                tool_post.result = content.clone();
                tool_post.duration = Some(tool_start.elapsed());
                fire_post(&middleware, &tool_post);

                // One tool turn per result (ADR-100 MSG-009).
                turns.push(Message {
                    role: "tool".into(),
                    parts: vec![Part::ToolResult(ToolResult {
                        tool_use_id: call.id,
                        content,
                    })],
                    provider_turn: None,
                });
            }
        }

        Err(Error::Unsupported(format!(
            "max tool iterations ({}) reached",
            self.max_tool_iterations
        )))
    }

    fn find_tool(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|tool| tool.name == name)
    }
}
