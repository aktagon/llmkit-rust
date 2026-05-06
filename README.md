# llmkit (Rust)

Unified async LLM client library. One API for OpenAI, Anthropic, Google, Bedrock, and 23 other providers.

Streaming, batches, tool-calling agents, image generation, and middleware. Async via `tokio` + `reqwest`.

## Install

```toml
[dependencies]
llmkit = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Rust 2021, MSRV 1.75. Async runtime is `tokio`.

## Quick start

```rust
use llmkit::{prompt, PromptOptions, Provider, ProviderName, Request};

#[tokio::main]
async fn main() -> Result<(), llmkit::Error> {
    let provider = Provider::new(ProviderName::Anthropic, std::env::var("ANTHROPIC_API_KEY").unwrap());
    let response = prompt(
        &provider,
        &Request::new("Say hi").with_system("Be concise."),
        PromptOptions::new().temperature(0.3),
    )
    .await?;
    println!("{}", response.text);
    println!("{} input tokens", response.usage.input);
    Ok(())
}
```

## Streaming

```rust
use llmkit::{prompt_stream, PromptOptions, Provider, ProviderName, Request};

let provider = Provider::new(ProviderName::Openai, std::env::var("OPENAI_API_KEY").unwrap());
prompt_stream(
    &provider,
    &Request::new("Write a haiku about caching."),
    PromptOptions::new(),
    |chunk| print!("{chunk}"),
)
.await?;
```

## Tool-calling agent

```rust
use llmkit::{Agent, Provider, ProviderName, Tool};
use serde_json::json;

let mut agent = Agent::new(Provider::new(ProviderName::Anthropic, key));
agent.set_system("You can look up weather with the 'weather' tool.");
agent.add_tool(Tool::new(
    "weather",
    "Get weather for a city",
    json!({"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}),
    |args| {
        let city = args.get("city").and_then(|v| v.as_str()).unwrap_or("?");
        Ok(format!("It's sunny in {city}."))
    },
));
let response = agent.chat("What's the weather in Helsinki?").await?;
println!("{}", response.text);
```

## Image generation

Generate images from text, optionally conditioned on reference images. Currently supports Google's Nano Banana 2 (`gemini-3.1-flash-image-preview`) and Pro (`gemini-3-pro-image-preview`).

```rust
use llmkit::{generate_image, ImageOptions, ImageRequest, Provider, ProviderName};

let response = generate_image(
    &Provider::new(ProviderName::Google, key),
    &ImageRequest {
        prompt: "A nano banana dish in a fancy restaurant".into(),
        model: "gemini-3.1-flash-image-preview".into(),
        ..ImageRequest::default()
    },
    &ImageOptions {
        aspect_ratio: Some("16:9".into()),
        image_size: Some("2K".into()),
        ..ImageOptions::default()
    },
)
.await?;
std::fs::write("out.png", &response.images[0].data)?;
```

Per-model whitelists (aspect ratios + sizes) are enforced before any HTTP call.

| Model                 | Aspect ratios                                                               | Sizes           |
| --------------------- | --------------------------------------------------------------------------- | --------------- |
| Nano Banana 2 (Flash) | 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9, **1:4, 4:1, 1:8, 8:1** | 512, 1K, 2K, 4K |
| Nano Banana Pro       | 1:1, 2:3, 3:2, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9, 21:9                         | 1K, 2K, 4K      |

Up to 14 reference images per request.

## Batching

```rust
use llmkit::{prompt_batch, PromptOptions, Provider, ProviderName, Request};

let responses = prompt_batch(
    &Provider::new(ProviderName::Anthropic, key),
    &[
        Request::new("Translate hello to French"),
        Request::new("Translate hello to Spanish"),
    ],
    PromptOptions::new(),
)
.await?;
for response in responses {
    println!("{}", response.text);
}
```

`prompt_batch` is `submit_batch` + `wait_batch`. Use `submit_batch` to get a `BatchHandle` you can persist, then call `wait_batch(&handle, opts)` later.

## Caching

Opt in with `PromptOptions::new().caching()`. The mode is provider-specific and inferred:

- Anthropic — explicit `cache_control` on the system prompt.
- Google — pre-flight `cachedContents` create + reference.
- OpenAI — automatic prompt caching (no body change).

## Middleware

Register pre/post hooks around LLM requests, tool calls, cache creation, uploads, batch submits, and image generation. Pre-phase hooks can veto the operation by returning `Some(error)`; post-phase runs for observation only.

```rust
use std::sync::Arc;
use llmkit::{prompt, Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase, PromptOptions, Provider, ProviderName, Request};

let logger: MiddlewareFn = Arc::new(|ev: &Event| {
    if matches!(ev.phase, MiddlewarePhase::Post) {
        if let Some(usage) = &ev.usage {
            println!(
                "{:?}/{}: {} in, {} out",
                ev.op, ev.model, usage.input, usage.output
            );
        }
    }
    None
});

let mut options = PromptOptions::new();
options.middleware = vec![logger];
prompt(&provider, &Request::new("Hi"), options).await?;
```

`Agent::with_middleware(vec![...])` registers hooks for every LLM call and tool invocation the agent performs. A `ToolCall` veto aborts the chat with `llmkit::Error::MiddlewareVeto`.

## Providers

OpenAI, Anthropic, Google, Grok, Bedrock, OpenRouter, Groq, DeepSeek, Cohere, Mistral, Together, Fireworks, Cerebras, Doubao, Ernie, Moonshot, Qwen, Perplexity, SambaNova, Yi, AI21, Zhipu, MiniMax, Azure, Ollama, LM Studio, vLLM.

Each provider has a default model, auth scheme, and feature matrix discoverable via `llmkit::PROVIDERS`.

## API surface

Entry points: `prompt`, `prompt_stream`, `upload_file`, `prompt_batch`, `submit_batch`, `wait_batch`, `generate_image`, `Agent`.

Types: `Provider`, `Request`, `Response`, `Message`, `File`, `Tool`, `Usage`, `PromptOptions`, `ImageRequest`, `ImageResponse`, `ImageOptions`, `Event`, `MiddlewareFn`, `MiddlewareOp`, `MiddlewarePhase`.

Errors: `llmkit::Error` (covers `Validation`, `Api`, `Http`, `Json`, `MiddlewareVeto`, `Unsupported`).

## Architecture

The per-provider configuration in `src/providers/generated/` is generated; the runtime (HTTP, transforms, streaming, caching, batches, agent loop, image generation, middleware, SigV4 signing) is hand-coded with the help of AI.

## License

MIT.
