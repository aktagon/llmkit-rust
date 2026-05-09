# Changelog

All notable changes to the Rust SDK are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Deprecated

- Legacy free-function layer marked `#[deprecated]` (plan-018 D4, ADR-010). The bodies still exist as internal `pub(crate)` helpers; the `pub use` exports now route through thin deprecated wrappers that forward to those internals. Migrate to the typed builder reachable via `llmkit::builders::new_client(...)`:

  ```rust
  use llmkit::builders::new_client;
  use llmkit::ProviderName;
  let c = new_client(ProviderName::Anthropic, key);
  let resp = c.text().system("...").temperature(0.7).prompt("hello").await?;
  ```

  - `c.text().<chain>.prompt(msg).await` — replaces `prompt`.
  - `c.text().<chain>.stream(msg, callback).await` — replaces `prompt_stream`; callback signature unchanged.
  - `c.image().model(id).<chain>.generate(msg).await` — replaces `generate_image`.
  - `c.upload().path(p).run().await` — replaces `upload_file`.
  - `c.text().<chain>.batch(prompts).await` / `.submit_batch(prompts).await?.wait().await` — replaces the batch trio. The `BatchHandleExt` trait provides `.wait()` on the existing `BatchHandle` value.
  - `c.agent().<chain>.prompt(&mut self, msg).await` — replaces `Agent::new(...)` + `.chat(msg)`.

  The `#[deprecated]` shims are scheduled for removal once `rust/tests/{prompt,image}.rs` (1700 LOC) are hand-ported to the typed-builder.

### Added

- ADR-011 chain-field propagation lint integrated into `make check`.
- All eight sampling/decoding chain methods (`top_p`, `top_k`, `frequency_penalty`, `presence_penalty`, `seed`, `stop_sequences`, `thinking_budget`, `reasoning_effort`) now thread through to `PromptOptions` for both `*Text` and `*Agent`. They had been silently dropping since plan-016 phase 2b.

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
