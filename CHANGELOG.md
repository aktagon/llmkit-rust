# Changelog

All notable changes to the Rust SDK are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.0.1] — 2026-07-19

### Security

- Fixed: an API key could appear in error output. On a failed request to a query-parameter-authenticated provider (Google), the transport error's `Display` embedded the full request URL — which carries the key as a `?key=` query parameter — so logging the error could leak the key. The URL is now stripped from transport errors.

## [2.0.0] — 2026-07-19

### Breaking

- Clean async-job API (ADR-064). Batch is now a single async terminal on the `text` builder — `c.text().<chain>.batch(...).await` returns a `BatchHandle` (batch is a text execution mode, parallel to `stream`), and `handle.wait().await` resolves the ordered results. The old two-terminal surface is collapsed: the blocking `batch` (which returned `Vec<Response>`) and `submit_batch` are both gone — `batch` now returns the handle. The handle also `impl IntoFuture`, so `c.text().<chain>.batch(...).await?.await?` works. Migration: `c.text().<chain>.submit_batch(...)` → `c.text().<chain>.batch(...)`; the old blocking form → `let h = c.text().<chain>.batch(...).await?; h.wait().await?`.

### Added

- Typed telemetry error kind (ADR-071). The middleware `Event` carries a typed `err_type` set structurally from the error, and the OTLP span's `error.type` attribute now derives from it rather than from string classification of the message. Additive.

### Fixed

- Streamed OpenAI usage is no longer `0`: the SDK opts into `stream_options.include_usage` per provider (OpenAI), so streamed calls report real input/output token counts (BUG-028).
- A batch with an errored or unparseable result line now returns the successful subset instead of discarding the whole batch (HANDOFF-036 A1).
- Image and file input Parts are carried through the batch request envelope.
- The `models_list` middleware op now fires real client hooks (HANDOFF-036 A3).
- `with_capability(...)` now filters the scoped provider list (HANDOFF-036 A4).
- A malformed 2xx speech-generation body is now a typed decoding error instead of silent empty audio (HANDOFF-036 A5).
- The per-request `anthropic-beta` header is sent on batch submit, so a file-referencing batch item no longer 400s.

## [1.2.1] — 2026-07-11

### Fixed

- `providers::info(ProviderName::OpenAI).browser_callable` is now `false` (was `true`). OpenAI's API host answers the CORS preflight but omits `Access-Control-Allow-Origin` on the actual response, so a browser cannot read it directly. The flag now reflects the real response, not the preflight — a browser app that reads it to decide direct-call-vs-proxy will correctly route OpenAI through a proxy. Google stays `true` (its responses carry the header). No API change.

## [1.2.0] — 2026-07-06

### Added

- Inline image input on the text/prompt path (ADR-060). `c.text().image(mime, bytes).prompt(...)` now sends the image as the provider's native vision block on all four chat wire shapes (Anthropic, OpenAI, Google, Bedrock). Bytes-based, so it works with no filesystem. Resolves ADR-008 OQ-2 for the image modality; additive (the `.image(...)` builder method previously dropped the image on this path).

## [1.1.0] — 2026-06-09

### Added

- Video generation — `c.video().model(id).submit(prompt).await` returns a `VideoHandle`; `handle.wait().await` (or the free `wait_video(&handle, poll)`) polls until the job finishes and returns `VideoResponse { videos: Vec<VideoData>, usage, finish_reason, finish_message }`. Each `VideoData` carries `url`, `mime_type`, and `duration_seconds`. One provider so far: xAI Grok Imagine (`grok-imagine-video`), which delivers a temporary hosted URL — download it yourself.
- Music generation — `c.music().model(id).generate(prompt)` (or the free `generate_music(...)`) produces audio from a text prompt, with an optional `.lyrics(...)` chain method for models that support vocals. Returns `MusicResponse { audio: Vec<AudioData>, text, usage }` with decoded audio bytes. Three providers: Vertex Lyria 2 (`lyria-002`, instrumental WAV), Google Lyria 3 (`lyria-3-pro-preview` / `lyria-3-clip-preview`, MP3 with lyrics), and MiniMax (`music-2.6`). Instrumental-only models reject lyrics before the request is sent.
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
