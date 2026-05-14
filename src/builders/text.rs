//! Phase 3 slice 1 — wires Text::prompt against legacy `prompt`.
//!
//! Codegen-emitted `Text::prompt` delegates to `text_prompt(self, msg)`
//! (see RUST_BUILDER_SKIP_TERMINALS in codegen/generate.py).

use crate::error::Error;
use crate::image::Part;
use crate::options::PromptOptions;
use crate::types::{Message, Provider, Request, Response};

use super::Text;

pub(super) fn build_provider(b: &Text) -> Provider {
    Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: b.model.clone(),
        base_url: b.client.provider.base_url.clone(),
    }
}

pub(super) fn build_request(b: &Text, final_text: &str) -> Request {
    let mut req = Request::default();
    if let Some(ref s) = b.system {
        req.system = Some(s.clone());
    }

    // Concatenate accumulated text Parts + final prompt.
    let mut user_text = String::new();
    for part in &b.parts {
        if let Part::Text(t) = part {
            user_text.push_str(t);
        }
    }
    user_text.push_str(final_text);

    if !b.history.is_empty() {
        let mut msgs: Vec<Message> = b.history.clone();
        if !user_text.is_empty() {
            msgs.push(Message {
                role: "user".to_string(),
                content: user_text,
            });
        }
        req.messages = msgs;
    } else if !user_text.is_empty() {
        req.user = Some(user_text);
    }
    if let Some(ref s) = b.schema {
        req.schema = Some(s.clone());
    }
    req
}

pub(super) fn build_options(b: &Text) -> PromptOptions {
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
    if !b.middleware.is_empty() {
        opts.middleware = b.middleware.clone();
    }
    if !b.safety_settings.is_empty() {
        opts.safety_settings = b.safety_settings.clone();
    }
    opts
}

pub async fn text_prompt(b: Text, msg: impl Into<String>) -> Result<Response, Error> {
    let final_text: String = msg.into();
    let provider = build_provider(&b);
    let request = build_request(&b, &final_text);
    let options = build_options(&b);
    crate::prompt(&provider, &request, options).await
}
