//! Phase 3 slice 2c — wires Agent::prompt + Agent::reset against the
//! legacy `Agent` struct.
//!
//! Stateful builder pattern. The typed-builder `Agent` carries a
//! private `state: Option<AgentState>` field that wraps a live
//! legacy `Agent`. First `.prompt()` lazily constructs it from
//! chained config; subsequent calls reuse it so history accumulates.
//!
//! Chain immutability: chain methods consume `mut self` and return
//!
//! RUST_BUILDER_POST_MUTATION["Agent"]). So a forked clone via
//! `bot.system("new")` gets fresh state — but Rust's ownership rules
//! mean the original is moved, so there's no accidental reuse of the
//! parent post-fork. To keep a parent + a fork, call `.clone()` on the
//!
//!
//! Receiver: terminals on this builder use `&mut self` instead of
//! consuming `self` (override in RUST_BUILDER_TERMINAL_RECEIVER) so
//! repeated `.prompt()` calls share state without destroying the
//! builder.

use crate::agent::Agent as LegacyAgent;
use crate::error::Error;
use crate::options::SendOptions;
use crate::structs::{Request, Response, ToolDefinition};
use crate::types::{Provider, Tool};

use super::Agent;

/// Internal handle wrapping the agent runtime, carried by the
/// crate-internal `Agent.state` field.
pub(crate) struct AgentState {
    agent: LegacyAgent,
}

impl AgentState {
    /// Test-only constructor used by the state-forking contract test
    /// in `src/builders/internal_tests.rs`. Hidden from docs and
    /// gated to test builds (consumers shouldn't see it; rustc would
    /// otherwise flag it dead_code in non-test profiles).













    /// ADR-020 HIST-004: project the wrapped legacy agent's internal
    /// history into the public Message shape. Called by the typed
    ///
    /// RUST_BUILDER_EXTRA_METHODS).
    pub fn public_messages(&self) -> Vec<crate::structs::Message> {
        self.agent.public_messages()
    }

    /// HANDOFF-105 F5, owner decision 2: the turns of the last failed call.
    /// Called by the typed builder's `bot.unsaved_turns()` accessor (emitted
    ///
    pub fn unsaved_turns(&self) -> Vec<crate::structs::Message> {
        self.agent.unsaved_turns()
    }
}

/// What the request sends of each tool: its name, description and schema
/// (ADR-098 RQC-018).
fn tool_definitions(tools: &[Tool]) -> Vec<ToolDefinition> {
    tools
        .iter()
        .map(|t| ToolDefinition {
            name: t.name.clone(),
            description: t.description.clone(),
            schema: t.schema.clone(),
        })
        .collect()
}

/// Constructs the agent runtime from the chained config: the Request each
/// turn sends, the tools, and the send options. The builder's model goes into
/// `Request.model`, so the provider carries none. A TTL that is not positive
/// whole seconds is a validation error
/// ([`check_cache_ttl`](crate::caching::check_cache_ttl)) and leaves no state.
fn init_agent(b: &Agent) -> Result<AgentState, Error> {
    crate::caching::check_cache_ttl(b.cache_ttl)?;
    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
        timeout: b.client.provider.timeout,
    };

    let request = Request {
        model: b.model.clone().unwrap_or_default(),
        system: b.system.clone().unwrap_or_default(),
        tools: tool_definitions(&b.tools),
        max_tokens: b.max_tokens,
        temperature: b.temperature,
        top_p: b.top_p,
        top_k: b.top_k,
        stop: b.stop_sequences.clone(),
        seed: b.seed,
        frequency_penalty: b.frequency_penalty,
        presence_penalty: b.presence_penalty,
        reasoning_effort: b.reasoning_effort.clone(),
        thinking_budget: b.thinking_budget,
        thinking_type: b.thinking_type.clone(),
        safety_settings: b.safety_settings.clone(),
        ..Request::default()
    };
    let options = SendOptions {
        caching: b.caching,
        cache_ttl: b.cache_ttl,
        middleware: b.middleware.clone(),
        raw: b.raw,
    };

    let mut agent = LegacyAgent::new(provider, request, b.tools.clone(), options);
    if let Some(n) = b.max_tool_iterations {
        agent.set_max_tool_iterations(n as usize);
    }
    // ADR-020 HIST-007: seed the legacy agent's internal history
    // from the chain's typed Message list.
    if !b.history.is_empty() {
        agent.seed_history(b.history.clone());
    }
    Ok(AgentState { agent })
}

pub(crate) async fn agent_prompt(b: &mut Agent, msg: impl Into<String>) -> Result<Response, Error> {
    if b.state.is_none() {
        b.state = Some(init_agent(b)?);
    }
    let state = b.state.as_mut().expect("state initialized above");
    state.agent.chat(msg).await
}

/// Clears state. Chain config (system, tools, max-tokens, ...) is
/// preserved on the typed builder; next `.prompt()` re-runs
/// `init_agent`. Deliberately doesn't call `LegacyAgent::reset()`,
/// which clears tools too — the typed builder's own `tools` Vec
/// re-supplies them on re-init.
pub(crate) fn agent_reset(b: &mut Agent) {
    b.state = None;
}
