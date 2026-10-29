//! Phase 3 slice 1 — wires Text::prompt against legacy `prompt`.
//!
//!
//!

use crate::error::Error;
use crate::image::Part;
use crate::options::PromptOptions;
use crate::structs::{Message, Response};
use crate::types::{Provider, Request};

use super::Text;

pub(super) fn build_provider(b: &Text) -> Provider {
    Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: b.model.clone(),
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
        timeout: b.client.provider.timeout,
    }
}

pub(super) fn build_request(b: &Text, final_text: &str) -> Request {
    let mut req = Request::default();
    if let Some(ref s) = b.system {
        req.system = Some(s.clone());
    }

    // The final user turn (ADR-100 MSG-004), ordered so every request
    // golden keeps its bytes: the files, then the image parts, then one text
    // part joining the text parts and final_text with newlines (the
    // text-parts-openai request-wire golden holds every SDK to the join). The
    // text part is left out when there is no text. It follows the history
    // (BUG-108).
    let mut turn: Vec<Part> = b.files.iter().cloned().map(Part::File).collect();
    let mut texts: Vec<&str> = Vec::new();
    for part in &b.parts {
        match part {
            Part::Text(t) if !t.is_empty() => texts.push(t),
            Part::Image(_) => turn.push(part.clone()),
            _ => {}
        }
    }
    if !final_text.is_empty() {
        texts.push(final_text);
    }
    if !texts.is_empty() {
        turn.push(Part::Text(texts.join("\n")));
    }

    req.messages = b.history.clone();
    if !turn.is_empty() {
        req.messages.push(Message {
            role: "user".into(),
            parts: turn,
            provider_turn: None,
        });
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
    if let Some(ref v) = b.protocol {
        if !v.is_empty() {
            opts.protocol = Some(v.clone());
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
    if b.raw {
        opts.raw = true;
    }
    opts
}

pub(crate) async fn text_prompt(b: Text, msg: impl Into<String>) -> Result<Response, Error> {
    let final_text: String = msg.into();
    let provider = build_provider(&b);
    let request = build_request(&b, &final_text);
    let options = build_options(&b);
    crate::prompt(&provider, &request, options).await
}
