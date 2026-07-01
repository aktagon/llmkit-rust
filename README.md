# llmkit (Rust)

One Rust API for Anthropic, OpenAI, Google, and 20+ other providers — including local models through Ollama and vLLM. Switch providers without rewriting your request.

Async, built on `tokio` and `reqwest`.

Also available for Go, TypeScript, and Python.

<p align="center">
  <img src="https://raw.githubusercontent.com/aktagon/llmkit-rust/master/assets/logos/llmkit-languages.svg" alt="Go, TypeScript, Python, Rust" height="26">
</p>
<p align="center">
  <img src="https://raw.githubusercontent.com/aktagon/llmkit-rust/master/assets/logos/llmkit-providers.svg" alt="Anthropic, OpenAI, Google, and 20+ more providers" height="26">
</p>

## Install

```toml
[dependencies]
llmkit = "1.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Quick Start

```rust
use llmkit::builders::anthropic;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let c = anthropic(std::env::var("ANTHROPIC_API_KEY")?);
    let resp = c.text()
        .system("Be concise.")
        .temperature(0.3)
        .prompt("Why is the sky blue?")
        .await?;

    println!("{}", resp.text);
    println!("{} input tokens", resp.usage.input);
    Ok(())
}
```

The typed builder is the only public surface as of v1.0.0. One mental model — `client.<capability>().<chain>.<terminal>` — across every capability.

Runnable counterparts to every code block below live in [`examples/`](./examples/) and are exercised by `tests/examples.rs` against a mock HTTP server, so the call shapes shown here are guaranteed to execute against the real builder surface.

## Providers

Per-provider factory functions in `llmkit::builders`:

```
ai21       anthropic  azure      bedrock    cerebras   cohere
deepseek   doubao     ernie      fireworks  google     grok
groq       jan        llamacpp   lmstudio   minimax    mistral
moonshot   ollama     openai     openrouter perplexity qwen
sambanova  together   vertex     vllm       yi         zhipu
```

Or use the generic `new_client(ProviderName::OpenAI, key)`. 30 providers, 4 API shapes (OpenAI-compatible, Anthropic Messages, Google Generative AI, AWS Bedrock Converse). Bedrock auth uses SigV4; other providers use API-key auth.

## API

### Text — one-shot prompt

```rust
let resp = c.text()
    .system("You are helpful")
    .temperature(0.7)
    .max_tokens(200)
    .prompt("What is 2+2?")
    .await?;

println!("{}", resp.text);              // "4"
println!("{}", resp.usage.input);       // prompt tokens
println!("{}", resp.usage.output);      // completion tokens
println!("{}", resp.usage.cache_read);  // tokens served from cache
println!("{}", resp.usage.cache_write); // tokens written to cache (Anthropic explicit)
println!("{}", resp.usage.reasoning);   // internal reasoning tokens (OpenAI o-series, Gemini 2.5+)
```

Capability-scoped fields (`cache_read`, `cache_write`, `reasoning`) are zero when the provider doesn't report them separately.

### Stream — callback + trailing handle

Rust's stream surface is callback-based. The callback fires for each chunk; the awaited terminal returns the final `Response` with token counts.

<!-- llmkit:include rust/examples/streaming.rs#stream -->
```rust
let resp = c
    .text()
    .system("Be brief")
    .stream("Tell me a joke", |chunk| {
        print!("{}", chunk);
        let _ = std::io::stdout().flush();
    })
    .await?;

println!();
println!("Usage: {} in / {} out", resp.usage.input, resp.usage.output);
```

The callback shape is the trailing-handle pattern from the other SDKs expressed in callback form: callback receives chunks (≡ iterator), the returned `Result<Response>` is the trailing handle (≡ `stream.response()` in TS/Python). The `impl Stream<Item = ...>` variant from `futures` would mirror the other SDKs visually but pulls in an extra dependency we chose to avoid.

### Agent — tool loop

```rust
use llmkit::Tool;

let add = Tool::new(
    "add",
    "Add two numbers",
    serde_json::json!({
        "type": "object",
        "properties": {
            "a": {"type": "number"},
            "b": {"type": "number"},
        },
    }),
    |args| Ok((args["a"].as_f64().unwrap() + args["b"].as_f64().unwrap()).to_string()),
);

let mut bot = c.agent()
    .system("You are a calculator.")
    .add_tool(add)
    .max_tool_iterations(5);

let resp = bot.prompt("What is 2+3?").await?;
println!("{}", resp.text);
```

`*Agent` is **stateful** — repeated `bot.prompt(...)` calls accumulate history. Chain methods (`.system(...)`, `.add_tool(...)`) consume `self` and produce a fresh-state clone, so a forked builder gets a fresh conversation. `bot.reset()` clears state without dropping chained config.

Tool dispatch covers Anthropic `tool_use`, OpenAI `tool_calls`, Google `functionCall`, and Bedrock Converse `toolUse`.

### Image — text-to-image and edit

Supports Google's Nano Banana 2 (`gemini-3.1-flash-image-preview`) and Pro (`gemini-3-pro-image-preview`); OpenAI's `gpt-image-2`, `gpt-image-1.5`, `gpt-image-1`, and `gpt-image-1-mini`; xAI's `grok-imagine-image-quality`; Google Cloud Vertex AI's Imagen 3 / Imagen 4 (`imagen-3.0-generate-002`, `imagen-3.0-fast-generate-001`, `imagen-4.0-generate-preview-06-06`).

```rust
use llmkit::builders::google;

let c = google(std::env::var("GOOGLE_API_KEY")?);
let img = c.image()
    .model("gemini-3.1-flash-image-preview")
    .aspect_ratio("16:9")
    .image_size("2K")
    .generate("A nano banana dish, studio lighting")
    .await?;

std::fs::write("out.png", &img.images[0].bytes)?;
```

For compositional editing, chain `.text(...)` and `.image(mime, bytes)` to interleave references with descriptions:

```rust
c.image()
    .model("gemini-3.1-flash-image-preview")
    .text("Person:")
    .image("image/png", person_bytes)
    .text("Outfit:")
    .image("image/png", outfit_bytes)
    .generate("Generate the person wearing the outfit.")
    .await?;
```

Aspect ratios and sizes validate against a per-model whitelist before the HTTP request. Empty whitelists mean "no client-side check; pass through" — providers like OpenAI accept arbitrary sizes within documented bounds (max edge ≤3840, both edges multiples of 16, ratio ≤3:1, total pixels 655K–8.3M), so the SDK trusts the API boundary instead of carrying a stale list.

For OpenAI, the chain dispatches automatically — no image parts hits `/v1/images/generations` (JSON), one or more image parts hits `/v1/images/edits` (multipart/form-data with one `image[]` field per reference, in caller order).

Provider knobs are typed chain methods on the `Image` builder:

| Method               | Provider support            | Wire field       |
| -------------------- | --------------------------- | ---------------- |
| `.quality(s)`        | OpenAI gpt-image-\*         | `quality`        |
| `.output_format(s)`  | OpenAI gpt-image-\*         | `output_format`  |
| `.background(s)`     | OpenAI gpt-image-\*         | `background`     |
| `.count(n)`          | OpenAI + xAI Grok           | `n`              |
| `.mask(mime, bytes)` | OpenAI gpt-image-\* (edits) | multipart `mask` |

The chain validates per provider — calling `.quality(...)` on a Google or xAI builder returns `Err(Validation { ... })` immediately, no HTTP round-trip. Knobs without typed methods (OpenAI: `output_compression`, `moderation`) remain reachable via `.extra_fields(...)`, which is unvalidated and freeform.

```rust
use llmkit::builders::openai;

let c = openai(std::env::var("OPENAI_API_KEY")?);
let resp = c.image()
    .model("gpt-image-2")
    .image_size("1024x1024")
    .quality("high")
    .count(4)
    .generate("A red circle on a white background")
    .await?;
```

OpenAI gpt-image-\* models require organization verification — see [platform.openai.com/docs/guides/your-data#organization-verification](https://platform.openai.com/docs/guides/your-data#organization-verification).

Up to 14 reference images per Google request, 16 per OpenAI request.

#### Vertex AI Imagen (Google Cloud)

Vertex Imagen uses the `:predict` endpoint family and OAuth bearer auth instead of API keys. The SDK takes a bearer token (string); caller manages OAuth refresh externally (e.g. `gcloud auth print-access-token`, service-account JSON, or workload identity).

```rust
use llmkit::builders::vertex;

// Caller substitutes {project_id} and {location} before passing the URL.
let base_url = "https://us-central1-aiplatform.googleapis.com\
    /v1/projects/my-gcp-project/locations/us-central1/publishers/google/models";

let c = vertex(std::env::var("VERTEX_BEARER_TOKEN")?).base_url(base_url);

let resp = c
    .image()
    .model("imagen-3.0-generate-002")
    .aspect_ratio("16:9")
    .count(2)
    .generate("A red circle")
    .await?;
```

Edit-mode (single image into `instances[0].image`) and inpainting (`.mask(mime, bytes)` into `instances[0].mask.image`) work the same way. Imagen-specific knobs like `negativePrompt` and `safetySetting` are reachable through `.extra_fields(...)` — they spread into the request's `parameters` block. Vertex's `:predict` response does not carry token counts; `resp.usage` stays zero.

### Music — text-to-music

Generate audio from a text prompt via the typed-builder chain on `c.music()` (or the free `generate_music(...)`). Decoded audio bytes come back on `resp.audio[0].bytes`. Models that support vocals take lyrics via `.lyrics(...)` (use section tags like `[verse]`); instrumental-only models reject lyrics before the request is sent.

<!-- llmkit:include rust/examples/music.rs#music -->
```rust
let resp = c
    .music()
    .model("lyria-002")
    .generate("a calm instrumental, warm piano and soft strings")
    .await?;

if let Some(first) = resp.audio.first() {
    std::fs::write("out.wav", &first.bytes)?;
    println!("wrote {} bytes to out.wav ({})", first.bytes.len(), first.mime_type);
} else {
    println!("no audio returned");
}
```

Models with vocals take lyrics via `.lyrics(...)`:

```rust
let song = c.music().model("lyria-3-pro-preview")
    .lyrics("[verse] neon lights").generate("dream pop, 90 bpm").await?;
```

| Provider | Model(s)                                      | Lyrics | Output     |
| -------- | --------------------------------------------- | ------ | ---------- |
| Vertex   | `lyria-002`                                   | no     | WAV (~30s) |
| Google   | `lyria-3-pro-preview`, `lyria-3-clip-preview` | yes    | MP3        |
| MiniMax  | `music-2.6`                                   | yes    | MP3        |

### Video — text-to-video

Generate video from a text prompt. Video generation is asynchronous: `submit` returns a handle immediately, and `handle.wait()` polls until the job finishes. The result carries a temporary hosted URL on `resp.videos[0].url` — download it yourself.

<!-- llmkit:include rust/examples/video.rs#video -->
```rust
let handle = c
    .video()
    .model("grok-imagine-video")
    .submit("a slow cinematic drone shot flying over snow-capped alpine peaks at golden hour")
    .await?;

let resp = handle.wait().await?;

if let Some(first) = resp.videos.first() {
    println!(
        "url={} duration={}s mime={}",
        first.url, first.duration_seconds, first.mime_type
    );
} else {
    println!("no video returned");
}
```

| Provider | Model                | Delivery |
| -------- | -------------------- | -------- |
| Grok     | `grok-imagine-video` | URL      |

### Safety Settings

Control content filtering for Gemini providers. `safety_settings` applies to text
generation, streaming, agents, and Gemini image generation. `safety_filter` applies
to Vertex Imagen only.

```rust
use llmkit::builders::{google, vertex};
use llmkit::types::{
    SafetySetting,
    HARM_CATEGORY_DANGEROUS_CONTENT,
    HARM_CATEGORY_HARASSMENT,
    HARM_BLOCK_THRESHOLD_NONE,
    HARM_BLOCK_THRESHOLD_HIGH_ONLY,
    IMAGE_SAFETY_FILTER_BLOCK_FEW,
};

// Gemini text or agent
let c = google(std::env::var("GOOGLE_API_KEY")?);
let resp = c
    .text()
    .safety_settings(vec![
        SafetySetting { category: HARM_CATEGORY_DANGEROUS_CONTENT.into(), threshold: HARM_BLOCK_THRESHOLD_NONE.into() },
        SafetySetting { category: HARM_CATEGORY_HARASSMENT.into(), threshold: HARM_BLOCK_THRESHOLD_HIGH_ONLY.into() },
    ])
    .prompt("Write a story")
    .await?;

// Vertex Imagen
let vc = vertex(std::env::var("VERTEX_BEARER_TOKEN")?);
let img = vc
    .image()
    .model("imagen-3.0-generate-002")
    .safety_filter(IMAGE_SAFETY_FILTER_BLOCK_FEW)
    .generate("A landscape")
    .await?;
```

`safety_settings` on Vertex Imagen and `safety_filter` on non-Imagen providers return
`Err(ValidationError)`. The `HARM_CATEGORY_*`, `HARM_BLOCK_THRESHOLD_*`, and
`IMAGE_SAFETY_FILTER_*` constants cover all documented values; raw strings also work.

### Upload — Path or Bytes

```rust
use llmkit::builders::openai;

let c = openai(std::env::var("OPENAI_API_KEY")?);

// from a path
let file = c.upload().path("./data.pdf").run().await?;

// from bytes (filename required)
let file2 = c.upload()
    .bytes(buf)
    .filename("report.pdf")
    .mime_type("application/pdf")
    .run()
    .await?;
```

### Batches

```rust
use llmkit::builders::BatchHandleExt;

let results = c.text()
    .system("Be brief")
    .batch(vec!["Translate hello to French".into(), "Translate hello to Spanish".into()])
    .await?;
for r in &results { println!("{}", r.text); }

// Or split:
let handle = c.text().submit_batch(prompts).await?;
let results = handle.wait().await?;
```

Both inline (Anthropic) and file-reference (OpenAI two-hop) flows are handled internally. Import the `BatchHandleExt` trait to call `.wait()` on the returned handle.

### Caching

```rust
// Anthropic — explicit cache_control wrap of the system prompt.
c.text().system(long_sys_prompt).caching().prompt("...").await?;

// OpenAI — automatic server-side caching (caching() is a hint; reads
// surface in resp.usage.cache_read regardless).
c.text().system(long_sys_prompt).caching().prompt("...").await?;

// Google — pre-flight POST creates a cachedContents resource, then
// the main call references it. Google requires ~1k+ tokens of system
// prompt:
c.text().system(big_sys_prompt).caching().prompt("...").await?;
```

The mode is provider-specific and inferred from the provider config. The default TTL for Google is 3600s.

### Model catalogue

`c.models()` and `c.providers()` cover model discovery in three modes. Runnable counterpart at [`examples/catalogue.rs`](./examples/catalogue.rs).

```rust
use llmkit::{Capability, Provider, ProviderName};

// 1. Compiled-in catalogue — synchronous, no HTTP.
let all = c.models().list();
let info = c.models().get("claude-opus-4-7");         // Option<ModelInfo>
let chat = c.models().with_capability(Capability::ChatCompletion).list();

// 2. Providers namespace.
c.providers().list();      // configured (credentials + /v1/models endpoint)
providers::list();         // every provider the SDK ships with (static, keyless)

// 3. Live + scoped HTTP.
let live = c.models().live().await;                   // LiveResult — fan-out
let p = Provider::new(ProviderName::Anthropic, "sk-...");
let scoped = c.models().provider(p.clone()).list().await?;
let raw = c.models().provider(p).raw().list().await?; // ModelInfo.raw populated
```

`live().await` calls every configured provider's `/v1/models` in parallel and aggregates results into `LiveResult.models` + a per-provider `LiveResult.errors` map (partial success is the normal case). `provider(p).raw().list()` opts into populating `ModelInfo.raw` with the provider-native record — useful when you need fields the universal `ModelInfo` does not carry (Anthropic's capability matrix, Google's `supportedGenerationMethods`, etc.).

## Options

Across every `*Text` / `*Agent` builder:

| Concept          | Method                 |
| ---------------- | ---------------------- |
| System prompt    | `.system(s)`           |
| Model override   | `.model(name)`         |
| Sampling         | `.temperature(t)`      |
| Token cap        | `.max_tokens(n)`       |
| Caching          | `.caching()`           |
| Middleware hooks | `.add_middleware(fns)` |
| Reasoning effort | `.reasoning_effort(l)` |
| Thinking budget  | `.thinking_budget(n)`  |

`*Text` adds `.history(msgs)` and `.schema(json)`; `*Agent` adds `.add_tool(t)` and `.max_tool_iterations(n)` and carries conversation history implicitly across `.prompt(...)` calls.

Sampling hyperparameters (`.top_p`, `.top_k`, `.seed`, `.frequency_penalty`, `.presence_penalty`, `.stop_sequences`) are validated per provider; unsupported options return `Error::Validation` rather than silently dropping.

The Image builder has a narrower set: `.model`, `.aspect_ratio`, `.image_size`, `.include_text`, `.text`, `.image`, `.middleware`. Upload: `.path`, `.bytes`, `.filename`, `.mime_type`, `.middleware`.

## Self-hosted endpoints

```rust
use llmkit::builders::openai;

let c = openai("anything").base_url("http://localhost:8080/v1");
```

Works for any OpenAI-compatible server (vLLM, LM Studio, Ollama, corporate gateways).

## Custom headers

Attach a custom HTTP header to every request — for example an authenticated gateway that needs its own auth header alongside the provider key. `add_header` is chainable and calls accumulate.

```rust
use llmkit::builders::anthropic;

let c = anthropic(&api_key)
    .base_url("https://gateway.example.com/anthropic")
    .add_header("cf-aig-authorization", format!("Bearer {gateway_token}"));
```

The custom header is sent in addition to the provider's auth header; it cannot override the provider auth header or the required version header.

## Middleware

Register pre/post hooks around LLM requests, tool calls, image generation, cache creation, uploads, and batch submits. Pre-phase middleware can veto by returning `Some(error)`; post-phase return values are discarded.

```rust
use std::sync::Arc;
use llmkit::builders::anthropic;
use llmkit::middleware::{Event, MiddlewareFn, MiddlewareOp, MiddlewarePhase};

// Observation: log token usage after every LLM request.
let log_usage: MiddlewareFn = Arc::new(|e: &Event| {
    if e.op == MiddlewareOp::LlmRequest && e.phase == MiddlewarePhase::Post {
        if let Some(u) = &e.usage {
            let ms = e.duration.map(|d| d.as_millis()).unwrap_or(0);
            println!(
                "{}/{}: {} in, {} out, {ms} ms",
                e.provider, e.model, u.input, u.output,
            );
        }
    }
    None
});

// Veto: abort if a daily budget is exceeded (pre-phase).
let limit = 5.00_f64;
let spent = Arc::new(std::sync::Mutex::new(0.0_f64));
let spent_for_gate = Arc::clone(&spent);
let budget_gate: MiddlewareFn = Arc::new(move |e: &Event| {
    if e.op == MiddlewareOp::LlmRequest
        && e.phase == MiddlewarePhase::Pre
        && *spent_for_gate.lock().unwrap() >= limit
    {
        let msg = format!("daily budget ${:.2} exceeded", limit);
        return Some(Box::<dyn std::error::Error + Send + Sync>::from(msg));
    }
    None
});

let c = anthropic("…");
let resp = c
    .text()
    .add_middleware(vec![budget_gate, log_usage])
    .prompt("…")
    .await?;
```

A pre-phase veto surfaces as `llmkit::Error::MiddlewareVeto(String)` carrying the formatted cause, so callers can discriminate it from transport or provider errors via `match err { Error::MiddlewareVeto(msg) => … }`. Middlewares fire in registration order; the first `Some(_)` pre-phase return aborts.

Wired at six sites: `Text.prompt` / `Agent::chat` LLM call (`op=LlmRequest`), `Agent` tool execution (`op=ToolCall`), `Image.generate` (`op=ImageGeneration`), `Upload.run` (`op=Upload`), `Text.submit_batch` (`op=BatchSubmit`), Google resource caching pre-flight (`op=CacheCreate`).

## Wire-format stability

`*Agent` history persists across process boundaries through two paired
functions:

```rust
let data = bot.save()?;                                  // String
// ...later, fresh process...
let bot = c.agent().system("...").tool(t).load(&data)?;
// returns Err(WireError::UnsupportedVersion { .. }) on mismatch
```

Or the free-function form for admin tooling:

```rust
use llmkit::{save_history, load_history};

let data = save_history(&msgs)?;
let msgs = load_history(&data)?;
```

The output is a JSON document with a `_v` integer envelope plus a
`messages` array. The version is tracked through
`WIRE_SCHEMA_VERSION`; the in-memory `Message` schema may evolve
additively under one version (new optional fields work on older
readers), but a renamed, removed, or retyped field requires a `_v`
bump and a migrator.

`save_history` / `load_history` are the ONLY guaranteed-stable
serialization path. `Message` does not implement `serde::Serialize`
today, so direct `serde_json::to_string` will not compile on a
`Message` value; even if that changes, the bytes would still lack the
`_v` envelope and `load_history` would reject them with
`WireError::MissingVersion`. Use the contract path for
anything that crosses a process boundary or a release.

## Mirror

This repo is a read-only mirror of a private monorepo. File issues here; code patches should target the private source via `christian@aktagon.com`.

## License

MIT
