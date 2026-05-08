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

use crate::agent::Agent as LegacyAgent;
use crate::error::Error;
use crate::options::PromptOptions;
use crate::types::{Provider, Response};

use super::Agent;

pub struct AgentState {
    agent: LegacyAgent,
}

impl AgentState {
    ///
    ///
    ///
    ///
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

///
///
///
///
///
pub fn agent_reset(b: &mut Agent) {
    b.state = None;
}
