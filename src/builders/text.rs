//! Phase 3 slice 1 — wires Text::prompt against legacy `prompt`.
//!
//!
//!

use crate::error::Error;
use crate::image::Part;
use crate::options::SendOptions;
use crate::structs::{Message, Request, Response};
use crate::types::Provider;

use super::Text;

/// The client's provider as the send paths read it. It carries no model: the
/// builder's model is `Request.model` (ADR-098 RQC-004).
pub(super) fn build_provider(b: &Text) -> Provider {
    Provider {
        name: b.client.provider.name,
        api_key: b.client.provider.api_key.clone(),
        model: None,
        base_url: b.client.provider.base_url.clone(),
        headers: b.client.provider.headers.clone(),
        timeout: b.client.provider.timeout,
    }
}

/// The Request the chain sends for one prompt: the model, every turn (the
/// final prompt last) and every request-shaping option (RQC-001).
pub(super) fn build_request(b: &Text, final_text: &str) -> Request {
    let mut req = Request::default();
    if let Some(ref m) = b.model {
        req.model = m.clone();
    }
    if let Some(ref s) = b.system {
        req.system = s.clone();
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
        req.schema = s.clone();
    }
    req.max_tokens = b.max_tokens.map(i64::from);
    req.temperature = b.temperature;
    req.top_p = b.top_p;
    req.top_k = b.top_k.map(i64::from);
    req.frequency_penalty = b.frequency_penalty;
    req.presence_penalty = b.presence_penalty;
    req.seed = b.seed;
    req.stop = b.stop_sequences.clone();
    req.thinking_budget = b.thinking_budget.map(i64::from);
    req.reasoning_effort = b.reasoning_effort.clone();
    req.safety_settings = b.safety_settings.clone();
    req
}

/// The chain's send options: what does not shape the request (RQC-002).
pub(super) fn build_options(b: &Text) -> SendOptions {
    SendOptions {
        caching: b.caching,
        cache_ttl: None,
        middleware: b.middleware.clone(),
        raw: b.raw,
    }
}

/// The chain's ADR-055 chat-protocol token; "" keeps the default.
pub(super) fn build_protocol(b: &Text) -> &str {
    b.protocol.as_deref().unwrap_or("")
}

pub(crate) async fn text_prompt(b: Text, msg: impl Into<String>) -> Result<Response, Error> {
    let final_text: String = msg.into();
    let provider = build_provider(&b);
    let request = build_request(&b, &final_text);
    let options = build_options(&b);
    crate::prompt(&provider, &request, build_protocol(&b), options).await
}
