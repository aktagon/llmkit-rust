# Changelog

All notable changes to the Rust SDK are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
