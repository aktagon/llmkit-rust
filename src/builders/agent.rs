//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!
//!

use crate::structs::Response;
use crate::agent::Agent as LegacyAgent;
use crate::error::Error;
use crate::options::PromptOptions;
use crate::types::{Provider};

use super::Agent;

///
///
///
#
pub struct AgentState {
    agent: LegacyAgent,
}

impl AgentState {
    ///
    ///
    ///
    ///
    #
    #
    pub(crate) fn placeholder(provider: crate::types::Provider) -> Self {
        Self {
            agent: LegacyAgent::new(provider),
        }
    }

    ///
    ///
    ///
    ///
    pub fn public_messages(&self) -> Vec<crate::structs::Message> {
        self.agent.public_messages()
    }
}

fn init_agent(b: &Agent) -> AgentState {
    let provider = Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: b.model.clone(),
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
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
    if b.raw {
        opts.raw = true;
    }

    let mut agent = LegacyAgent::new(provider);
    agent.set_options(opts);
    if let Some(n) = b.max_tool_iterations {
        agent.set_max_tool_iterations(n as usize);
    }
    if !b.middleware.is_empty() {
        agent.set_middleware(b.middleware.clone());
    }
    if let Some(ref s) = b.system {
        agent.set_system(s.clone());
    }
    for t in &b.tools {
        agent.add_tool(t.clone());
    }
    //
    //
    if !b.history.is_empty() {
        agent.seed_history(b.history.clone());
    }
    AgentState { agent }
}

pub(crate) async fn agent_prompt(b: &mut Agent, msg: impl Into<String>) -> Result<Response, Error> {
    if b.state.is_none() {
        b.state = Some(init_agent(b));
    }
    let state = b.state.as_mut().expect("state initialized above");
    state.agent.chat(msg).await
}

///
///
///
///
///
pub(crate) fn agent_reset(b: &mut Agent) {
    b.state = None;
}
