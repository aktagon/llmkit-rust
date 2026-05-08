//! Phase 3 slice 2c — wires Agent::prompt + Agent::reset against the
//! legacy `Agent` struct.
//!
//! Stateful builder pattern. The typed-builder `Agent` carries a
//! private `state: Option<AgentState>` field that wraps a live
//! legacy `Agent`. First `.prompt()` lazily constructs it from
//! chained config; subsequent calls reuse it so history accumulates.
//!
//! Chain immutability: chain methods consume `mut self` and return
//! `Self`, including `self.state = None` (codegen post-mutation hook,
//! RUST_BUILDER_POST_MUTATION["Agent"]). So a forked clone via
//! `bot.system("new")` gets fresh state — but Rust's ownership rules
//! mean the original is moved, so there's no accidental reuse of the
//! parent post-fork. To keep a parent + a fork, call `.clone()` on the
//! Client, not on the Agent (which doesn't derive Clone — see codegen).
//!
//! Receiver: terminals on this builder use `&mut self` instead of
//! consuming `self` (override in RUST_BUILDER_TERMINAL_RECEIVER) so
//! repeated `.prompt()` calls share state without destroying the
//! builder.

use crate::agent::Agent as LegacyAgent;
use crate::error::Error;
use crate::options::PromptOptions;
use crate::types::{Provider, Response};

use super::Agent;

pub struct AgentState {
    agent: LegacyAgent,
}

impl AgentState {
    /// Internal constructor — the typed-builder `Agent` lazily wraps a
    /// LegacyAgent on first .prompt(). Exposed as `pub` only so the
    /// state-forking integration test can manually populate state
    /// without touching a network. Stable only for that contract test.
    pub fn new(agent: LegacyAgent) -> Self {
        Self { agent }
    }
}

fn init_agent(b: &Agent) -> AgentState {
    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: b.model.clone(),
        base_url: b.client.provider.base_url.clone(),
    };

    let mut opts = PromptOptions::new();
    if let Some(n) = b.max_tokens {
        opts.max_tokens = Some(n);
    }
    if let Some(t) = b.temperature {
        opts.temperature = Some(t);
    }
    if b.caching {
        opts.caching = true;
    }

    let mut agent = LegacyAgent::new(provider);
    agent.set_options(opts);
    if !b.middleware.is_empty() {
        agent = agent.with_middleware(b.middleware.clone());
    }
    if let Some(ref s) = b.system {
        agent.set_system(s.clone());
    }
    for t in &b.tools {
        agent.add_tool(t.clone());
    }
    AgentState { agent }
}

pub async fn agent_prompt(b: &mut Agent, msg: impl Into<String>) -> Result<Response, Error> {
    if b.state.is_none() {
        b.state = Some(init_agent(b));
    }
    let state = b.state.as_mut().expect("state initialized above");
    state.agent.chat(msg).await
}

/// Clears state. Chain config (system, tools, max-tokens, ...) is
/// preserved on the typed builder; next `.prompt()` re-runs
/// `init_agent`. Deliberately doesn't call `LegacyAgent::reset()`,
/// which clears tools too — the typed builder's own `tools` Vec
/// re-supplies them on re-init.
pub fn agent_reset(b: &mut Agent) {
    b.state = None;
}
