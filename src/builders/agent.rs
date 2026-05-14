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

/// Internal handle wrapping the agent runtime. Exposed publicly only
/// because the typed-builder `Agent.state` field carries it; not part
/// of the v1.0 user-facing API. Hidden from docs.rs.
#[doc(hidden)]
pub struct AgentState {
    agent: LegacyAgent,
}

impl AgentState {
    /// Test-only constructor used by the state-forking contract test
    /// in `src/builders/internal_tests.rs`. Hidden from docs and
    /// flagged by name (`__test_only_*`) so external consumers know
    /// not to rely on it. Subject to change without a SemVer break.
    #[doc(hidden)]
    pub fn placeholder(provider: crate::types::Provider) -> Self {
        Self {
            agent: LegacyAgent::new(provider),
        }
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
    if let Some(v) = b.top_p {
        opts.top_p = Some(v);
    }
    if let Some(v) = b.top_k {
        opts.top_k = Some(v);
    }
    if let Some(v) = b.frequency_penalty {
        opts.frequency_penalty = Some(v);
    }
    if let Some(v) = b.presence_penalty {
        opts.presence_penalty = Some(v);
    }
    if let Some(v) = b.seed {
        opts.seed = Some(v);
    }
    if !b.stop_sequences.is_empty() {
        opts.stop_sequences = b.stop_sequences.clone();
    }
    if let Some(v) = b.thinking_budget {
        opts.thinking_budget = Some(v);
    }
    if let Some(ref v) = b.reasoning_effort {
        if !v.is_empty() {
            opts.reasoning_effort = Some(v.clone());
        }
    }
    if b.caching {
        opts.caching = true;
    }
    if !b.safety_settings.is_empty() {
        opts.safety_settings = b.safety_settings.clone();
    }

    let mut agent = LegacyAgent::new(provider);
    agent.set_options(opts);
    if let Some(n) = b.max_tool_iterations {
        agent.set_max_tool_iterations(n as usize);
    }
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
