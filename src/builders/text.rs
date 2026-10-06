//! Phase 3 slice 1 — wires Text::prompt against legacy `prompt`.
//!
//!
//!

use crate::structs::Response;
use crate::error::Error;
use crate::image::Part;
use crate::options::PromptOptions;
use crate::structs::Message;
use crate::types::{InputImage, Provider, Request};

use base64::Engine;

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

    // Join accumulated text Parts + final prompt with newlines (the
    // text-parts-openai request-wire golden holds every SDK to it); collect image Parts
    // into InputImage entries via base64 data URIs, preserving caller order
    // (mirror of go/text.go splitTextAndImages).
    let mut texts: Vec<&str> = Vec::new();
    let mut images: Vec<InputImage> = Vec::new();
    for part in &b.parts {
        match part {
            Part::Text(t) if !t.is_empty() => texts.push(t),
            Part::Image(m) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&m.bytes);
                images.push(InputImage {
                    url: format!("data:{};base64,{}", m.mime_type, b64),
                    mime_type: m.mime_type.clone(),
                    detail: String::new(),
                });
            }
            _ => {}
        }
    }
    if !final_text.is_empty() {
        texts.push(final_text);
    }
    let user_text = texts.join("\n");

    if !b.history.is_empty() {
        let mut msgs: Vec<Message> = b.history.clone();
        if !user_text.is_empty() {
            msgs.push(Message {
                role: "user".to_string(),
                content: user_text,
                ..Default::default()
            });
        }
        req.messages = msgs;
    } else if !user_text.is_empty() {
        req.user = Some(user_text);
    }
    if !b.files.is_empty() {
        req.files = b.files.clone();
    }
    if !images.is_empty() {
        req.images = images;
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
