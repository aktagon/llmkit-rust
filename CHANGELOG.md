# Changelog

All notable changes to the Rust SDK are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `Response.finish_reason` and `Response.finish_message` — provider stop signal + free-text explanation passed through verbatim on `client.text().prompt()`, the agent loop, and `client.text().stream(msg, callback)` (carried on the returned `Response` once the callback shape resolves). Examples: Anthropic `stop_reason`, OpenAI `choices[0].finish_reason`, Google `candidates[0].finishReason`. Default empty `String`; populated only when the provider response carries a signal. Streaming uses ADR-013's `event_name:json.path` locator — Anthropic captures from the `message_stop` event body; OpenAI/Grok/Google use last-non-empty-wins on the data frames; Google additionally filters `FINISH_REASON_UNSPECIFIED`. Bedrock Converse streaming is not yet wired.
- `ImageResponse.finish_reason` and `ImageResponse.finish_message` — same shape on `client.image().generate()`. Google populates both (including `IMAGE_OTHER` / `SAFETY` / `MAX_TOKENS` reasons that previously vanished into "no image returned"); Vertex Imagen surfaces `predictions[0].raiFilteredReason` as `finish_reason`; OpenAI Images API and xAI Grok have no equivalent fields and leave them empty.

## [1.0.0] — 2026-05-09

### Removed (Breaking)

- Legacy free-function layer deleted (plan 019, ADR-010, ADR-011). The bodies live on as `pub(crate)` helpers consumed by the typed-builder terminals; the public surface is now exclusively the typed builder reachable via `llmkit::builders::new_client(...)`:

  ```rust
  use llmkit::builders::new_client;
  use llmkit::ProviderName;
  let c = new_client(ProviderName::Anthropic, key);
  let resp = c.text().system("...").temperature(0.7).prompt("hello").await?;
  ```

  Migration map for the deleted symbols:
  - `prompt(...)` → `c.text().<chain>.prompt(msg).await`.
  - `prompt_stream(...)` → `c.text().<chain>.stream(msg, callback).await`; callback signature unchanged.
  - `generate_image(...)` → `c.image().model(id).<chain>.generate(msg).await`.
  - `upload_file(...)` → `c.upload().path(p).run().await` (or `.bytes(b).filename(n)`).
  - `prompt_batch(...)` / `submit_batch(...)` / `wait_batch(...)` → `c.text().<chain>.batch(prompts).await` / `.submit_batch(prompts).await?.wait().await`. The `BatchHandleExt` trait provides `.wait()` on the existing `BatchHandle` value.
  - `Agent::new(...)` + `.chat(msg)` → `c.agent().<chain>.prompt(msg).await` (note: `Agent::prompt` takes `&mut self`, not consuming `self`).

  The `#[deprecated]` shims that bridged this transition in 0.2.0 are gone; the previously-internal helpers (`prompt_internal`, `prompt_stream_internal`) have been renamed back to their canonical names (`prompt`, `prompt_stream`, both `pub(crate)`).

### Added

- ADR-011 chain-field propagation lint integrated into `make check`.
- All eight sampling/decoding chain methods (`top_p`, `top_k`, `frequency_penalty`, `presence_penalty`, `seed`, `stop_sequences`, `thinking_budget`, `reasoning_effort`) now thread through to `PromptOptions` for both `*Text` and `*Agent`. They had been silently dropping since plan-016 phase 2b.
- `Agent::max_tool_iterations(n)` chain method exposes the tool-loop depth cap (default 10) on the typed builder; calls `LegacyAgent::set_max_tool_iterations(n)` during state init.
- `Upload::bytes()` is now wired end-to-end alongside `path()`. New crate-internal `upload_bytes(provider, data, filename, mime_type, middleware)` helper matches the `reqwest::multipart::Part::bytes` idiom (`impl Into<Vec<u8>>` + `impl Into<String>` so callers pay no extra clone for owned data, and `&[u8]` / `&str` work via standard conversions). The path-based `upload_file` is unchanged; both delegate to a private `upload_with_data` helper that owns the middleware fire/post logic. `upload_bytes` is `pub(crate)` — the typed-builder is the only public surface.

### Documentation

- `*Text.stream(msg, callback)` doc comment updated to call out that Rust's callback shape is the trailing-handle pattern from the other SDKs expressed in callback form: callback receives chunks (≡ iterator), the returned `Result<Response>` is the trailing handle (≡ `stream.response()` / `stream.Response()`). The `impl Stream<Item = ...>` variant from `futures` would mirror the other SDKs visually but adds a third-party dependency the project's stdlib-only rule disallows. No code change.

### Removed

- `caching()` chain method on the `Image` builder. The legacy `generate_image` runtime never accepted a caching option, so the chain method had been a silent no-op.

## [0.2.0] — 2026-05-08

### Breaking

- `ImageRequest.reference_images` (and the `ImageInput` struct) is removed. Use `parts: Vec<Part>` instead, with the `Part::text(...)` and `Part::image(...)` constructors. Migration: `ImageRequest { prompt: "X".into(), reference_images: vec![ImageInput { mime_type: m, data: b }], .. }` becomes `ImageRequest { parts: vec![Part::text("X"), Part::image(m, b)], .. }`. Pure text-to-image callers using only `prompt: "X".into()` are unaffected.
- `ImageRequest` now requires exactly one of `prompt` or `parts` to be set (XOR). Both empty or both set returns `Error::Validation`.
- `Image` (the legacy text-generation vision-input struct on `Request.images`) is renamed to `InputImage`. Frees the `Part::Image(MediaRef)` enum variant.
- Multi-reference compositional generation now works by ordering the parts vec (e.g., `vec![Part::text("Person:"), Part::image(mime, ref_a), Part::text("Outfit:"), Part::image(mime, ref_b), Part::text("Generate ...")]`) — the wire shape preserves caller-controlled ordering. See ADR-008.

### Added

- `Part` (enum) and `MediaRef` (struct) types with `Part::text(impl Into<String>)` / `Part::image(impl Into<String>, impl Into<Vec<u8>>)` constructors. Universal multimodal atom shared across capabilities.

## [0.1.0] — 2026-05-06

First public release. Async via `tokio` + `reqwest`.

### Added

- `prompt(&provider, &request, options)` — one-shot LLM request.
- `prompt_stream(&provider, &request, options, callback)` — SSE streaming.
- `Agent` with `set_system`, `add_tool`, `with_middleware`, `add_middleware`, `chat`, `reset` — multi-turn tool-calling. Tool dispatch covers Anthropic `tool_use`, OpenAI `tool_calls`, Google `functionCall`, and Bedrock Converse `toolUse`.
- `upload_file(&provider, path, &middleware)` — multipart file upload.
- `prompt_batch` / `submit_batch` / `wait_batch` — batch lifecycle covering both inline-requests (Anthropic) and file-reference (OpenAI two-hop) flows.
- `generate_image(&provider, &request, &options)` — text-to-image and edit/composition with reference images. Google Nano Banana 2 + Pro. Per-model whitelist enforcement before any HTTP call.
- Three caching modes: automatic (OpenAI), explicit (Anthropic `cache_control`), resource (Google `cachedContents` pre-flight + reference).
- All four `SystemPlacement` modes including Bedrock Converse.
- Bedrock SigV4 signing, hand-rolled in `sigv4.rs`.
- Middleware runtime wired at six sites: `prompt` (LlmRequest), `apply_caching` (CacheCreate), `upload_file` (Upload), `submit_batch` (BatchSubmit), `generate_image` (ImageGeneration), `Agent::chat` (LlmRequest + ToolCall). Pre-phase veto + post-phase observation; `Error::MiddlewareVeto` discriminates from transport errors.
- 27 provider configs across 4 API shapes — all generated.
- `{model}` and `{region}` URL templating.
- Dotted-path option overrides nested correctly (e.g. Anthropic `thinking.budget_tokens` with sibling `type: "enabled"`).

### Tooling

- 33 unit tests against in-process `TcpListener` mock servers.
- `cargo build` clean, `cargo test` green.
