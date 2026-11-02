# Changelog

All notable changes to the Rust SDK are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [3.0.0] — 2026-11-01

### Added

- **BREAKING — `ModelInfo` carries what the SDK's catalogue states about a
  model.** `ModelInfo.image`, `video` and `speech` are `Some` for a model
  the SDK lists for that capability, and `None` otherwise. `ImageHints`
  holds the aspect ratios, sizes and whether the model takes image parts.
  `VideoHints` holds the resolutions, aspect ratios, maximum duration in
  seconds, and whether the model takes a seed image, a last frame or
  reference images. `SpeechHints` holds whether the model takes a speech
  style. Each support fact is an `Option<bool>`: `None` when no source
  states it. The values are hints: a request outside them is still sent.
  Code that builds a `ModelInfo` with a struct literal must set the three
  new fields.
- **`ImageHints::max_input_images` and `VideoHints::max_input_images` report
  the input-image limit.** Each is an `Option<i64>`: the most input images
  the catalogue lists for the model, and `None` when no source states a
  limit. `grok-imagine-video` reports `Some(1)`. Read it from the model's
  hints: `let p = Provider::new(ProviderName::OpenAI, key);`, then
  `let info = c.models().provider(p).get("gpt-image-1").await?;`, then
  `info.image.and_then(|i| i.max_input_images)`.
- **`ProviderError` and `ProviderInfo` are exported from the crate root.**
  `ProviderError` is the value type of `LiveResult.errors`, which no public
  path could name before. `ProviderInfo` is what
  `client.providers().list()` returns.
- **Video on `google` through Gemini Omni.** `gemini-omni-1.1-flash` makes a
  video from a text prompt on the Gemini Interactions API. It sends
  `.resolution(r)` and `.aspect_ratio(r)`. The SDK rejects a duration, a seed
  image, a last frame or reference images before send. The video bytes arrive
  in the reply. `VideoResponse.usage` reports input, output and reasoning
  tokens.
- **`VideoHandle::wait` deletes the job that Gemini Omni stores.** Once
  `wait` has the video, it sends a DELETE for the stored job.
  `client.video().keep_stored()` keeps the job. The handle records the
  choice in `VideoHandle.keep_stored`, so a resumed `wait` applies it. When
  the delete fails, `wait` returns the new `Error::VideoDelete { id, video,
  source }`. It holds the job id, the finished video and the delete error.
  Gemini Omni is the only provider that stores jobs, so `wait` on other
  providers is unchanged.
- **`Video::duration(s)`, `resolution(r)` and `aspect_ratio(r)` set the
  video length, resolution and aspect ratio.** OpenRouter, Grok and Vertex
  Veo send all three. Gemini Omni sends resolution and aspect ratio. Any
  other provider rejects a set option with `Error::Validation` before send.
  A value outside the model's hints is still sent.
- **`Video::last_frame(mime, data)` and `Video::add_reference_image(mime,
  data)`.** `last_frame` sets the frame the video ends on. Send it with a
  first frame from `.image(...)`; Vertex Veo and OpenRouter send it.
  `add_reference_image` adds an image of a person, character or product the
  video keeps. Vertex Veo takes up to 3, without a first or last frame, and
  only an 8-second duration. A provider whose request has no field for the
  input rejects it before send.
- **Multi-speaker speech and a speech style.** `Speech::add_speaker(name,
  voice)` declares a speaker, and `add_line(speaker, text)` adds a line it
  says. Declare exactly 2 speakers, and pass empty text to `generate`.
  Google takes multi-speaker requests. `add_styled_line(speaker, style,
  text)` gives one line a delivery style, such as "calm and relaxed".
  `style(s)` sets the style of a single-voice request. OpenAI and Google send
  a style; any other provider rejects it before send.
- **New provider capabilities.** Speech generation runs on `google` (Gemini
  TTS), `grok`, `minimax` and `openrouter`. Image and video generation run on
  `openrouter`.
- **`ProviderTurn` keeps the assistant turn as the provider wrote it.**
  `Response.provider_turn` and `Message.provider_turn` hold its JSON text in
  `wire` and its format in `wire_shape`. The Agent, and a loop that passes
  the message on, send the turn back unchanged on the next request.
  `save_history` and `load_history` keep it. A turn you write yourself has
  none, and the SDK builds it from the parts. Bedrock replies carry none.
- **`Response::created` (`Option<i64>`).** The Unix time the provider
  reports, read from `created` (Chat Completions) or `created_at`
  (Responses); `None` when the provider reports none.
- **`Response::tool_calls()`.** `text().request(...)` sends the request's
  tools and returns the model's tool calls without running them. Read them
  with `resp.tool_calls()`.
  `encode_response(ProviderName::Vertex, ...)` no longer returns
  `Error::Validation` for a finish reason; it writes none.
- **BREAKING — `finish_reason` on `ImageResponse`, `MusicResponse`,
  `SpeechResponse` and `VideoResponse` is renamed `provider_finish_reason`.**
  The value is unchanged: the provider's string, verbatim.
- **BREAKING — a batch item reports its outcome in `state` and `cause`.**
  Each `Response` from `BatchHandle::wait` or `poll` has `state`
  `Some(JobState::Succeeded)` or `Some(JobState::Failed)`; any other
  `Response` has `None`. A failed item has `cause: Some(JobFailure)` and no
  `finish_reason` or `finish_message`: the provider's failure status
  (`"errored"`, `"batch_expired"`, an error code) moved from `finish_reason`
  to `cause.status`, and its error message from `finish_message` to
  `cause.message`. A struct literal of `Response` must set both fields, or
  use `..Response::default()`.
- **BREAKING — `JobFailure::timed_out` is replaced by `JobFailure::code`.**
  `code` is a `JobFailureCode`: `Error`, `Expired`, `Cancelled`, `Timeout` or
  `Missing`, with `as_str()` for the string. `poll` reports it too: an OpenAI
  batch whose status is `expired` has `JobFailureCode::Expired`, and
  `cancelled` has `Cancelled`. `JobFailure` no longer implements `Default`.
  The poll deadline still returns `Error::PollTimeout`. `JobFailureCode` is
  exported from the crate root.
- Batch items carry `id` and `model` from the provider's response body.
- **BREAKING — `Error::Validation.field` is a `String`.** It was
  `&'static str`, which could not name a field of a decoded request body.
  A pattern such as `Error::Validation { field: "seed", .. }` becomes
  `Error::Validation { field, .. } if field == "seed"`.
- Every public data type without a floating-point field implements `Eq`:
  `AudioData`, `File`, `ImageData`, `MediaRef`, `Message`, `Part`,
  `ProviderError`, `ProviderTurn`, `SafetySetting`, `ToolCall`,
  `ToolDefinition`, `ToolResult`, `TranscriptSegment`, `VideoData` and
  `WireRequest`. `SafetySetting` keeps the `Eq` it had in 3.0.0; the others
  gain it.
- `Request` and `ToolDefinition` (name, description and input schema of a
  tool) are exported from the crate root.
- An empty system prompt (`.system("")`) sends no system field, as in the Go
  and TypeScript SDKs. It used to send an empty one.
- **BREAKING — `Part` has eight variants, and `Part::AudioBytes` is renamed.**
  The enum is `Text`, `Image`, `File`, `ToolCall`, `ToolResult`, `Lyrics`,
  `AudioUrl` and `Audio`. Local audio bytes moved from `Part::AudioBytes` to
  `Part::Audio`; `Part::audio_bytes(mime, bytes)` keeps its name. A `match`
  over `Part` must handle `File`, `ToolCall` and `ToolResult`.
- Image, music, video and transcription reject a part kind they do not accept,
  before any HTTP call, with `Error::Validation` on field `parts[i]` whose
  message names the kind, for example `image generation does not accept
  tool_call parts`. Image generation used to fold a lyrics part into its
  prompt text and ignore an audio part.
- A chat turn's rejected part fails on field `messages[i].parts[j]`, as in
  the Go and TypeScript SDKs, for example `assistant turn does not accept
  tool_result parts`. The field used to be `messages`, with the path in the
  message.
- `WireRequest` (the path, content headers and body of a chat request) is
  exported from the crate root.
- **BREAKING — `PromptOptions` is removed.** No public function took it
  except `wait_batch`. Every option of a chat call is a field of `Request`
  or a builder method (`.temperature(0.7)`).
- **BREAKING — `wait_batch(handle, poll)` takes two arguments.** The
  options argument is gone. Set `handle.raw` to get each raw result line.
- **BREAKING — the Agent and batch reject an option the provider does not
  take**, with `Error::Validation`, before any HTTP call. They used to send
  it.
- **BREAKING — a request with no turn fails** with `Error::Validation` on
  field `messages`. The field used to be `user`.
- `.reasoning_effort("")` sets an empty effort: it is validated and sent like
  any other value. It used to be dropped. Unset sends none.
- Text, stream, the Agent and batch put the builder's model in
  `Request.model`. A Google resource cache is created for that model.
- **BREAKING — a chat request without max tokens sends none, except to
  Anthropic.** Every request used to carry `max_tokens: 4096` (or the
  provider's key for it) when you set none, so a long answer was cut at 4096
  tokens. Now the provider's own default applies. The Anthropic Messages API
  rejects a request without `max_tokens`, so an Anthropic request still sends
  4096 when you set none. To keep a cap, set it: `.max_tokens(4096)`.

- `Response::id` and `Response::model` carry the response id and the model the
  provider reports. `prompt`, `stream`, the agent and `decode_response` fill
  them, and `encode_response` writes them back. Each is `None` when the
  response carries none, as on Bedrock. `model` can differ from the requested
  model: OpenAI returns the dated snapshot, and Google returns the served
  version.
- `Text::request(req)` sets the whole `Request` on the text builder: its model, system
  prompt, messages, tools, schema and options. It replaces what the chain set
  before it, the text, image and file parts included; a chain method after it
  changes its own field. A request from `decode_request` is sent unchanged by
  `.prompt("")`. Caching, its TTL, middleware, `raw` and
  `protocol` are kept.
- `Text::cache_ttl(seconds)` and `Agent::cache_ttl(seconds)` set how long
  the cache that `.caching()` creates lives. Google resource caching sends it on the cache-create
  request; without it, Google's default TTL (3600 s) applies. Other
  providers ignore it. Before, no chat call could set a
  TTL. The TTL is positive whole seconds:
  `cache_ttl(0)` fails the call with `Error::Validation` on field
  `cache_ttl`.
- `encode_request(provider, chat_wire_shape, &request)` encodes a chat
  request for a provider's wire shape and returns a `WireRequest`: the path
  with the model filled in, the content headers, and the JSON body. It needs
  no key and makes no network call. An empty `chat_wire_shape` selects the
  provider's default. A provider with no chat wire shape, such as AssemblyAI,
  fails with `Error::Validation` on field `chat_wire_shape`.
- `decode_request(provider, chat_wire_shape, &wire)` decodes an OpenAI Chat
  Completions request into a `Request`, the inverse of `encode_request`. It
  reads the model, a leading system turn, user and assistant turns with text
  content, and every option the provider takes. `max_tokens` and
  `max_completion_tokens` both set `max_tokens`. A bare-string `stop` is a
  one-item list, `"stream": false` is ignored, and a `developer` turn is the
  system turn for OpenAI and OpenRouter, whose APIs define that role; other
  providers reject it on the turn's role. An integer option takes an
  integral number in any notation, such as `4096.0`, and rejects a fraction.
  A number out of range, such as `1e999`, fails on its own field. Any other field, such as `"stream": true`, `tools`, `response_format` or a content list, fails with
  `Error::Validation` naming it, for example `messages[1].tool_calls`. Other
  wire shapes fail on field `chat_wire_shape`.

- **Builds and runs on wasm32-unknown-unknown.** The crate now compiles for
  the browser, and its requests no longer panic there. In the browser:
  - The client reads time from `performance.now()` and `Date.now()` and waits
    with `setTimeout`, so it runs in a window, a worker or Node. Native builds
    still use `std::time` and tokio.
  - Awaiting a `BatchHandle`, `TranscriptionHandle` or `VideoHandle` returns a
    future without `Send`, because browser futures are not `Send`. Native
    builds keep `Send`.
  - The client timeout works as on native builds. It bounds the wait for
    the response headers and for each next body chunk. On expiry the SDK
    aborts the fetch and returns `Error::Timeout`.
  - A `TextStream` yields each delta as its chunk arrives.
  - Reading a file from a path, AWS credentials from the environment, and the
    built-in HTTP telemetry exporter return an error.

- **Streaming with the Responses protocol.** A stream with `.protocol("responses")`
  now posts to `/v1/responses` and reads its events: text deltas, the finish
  status and token usage. It used to fail validation.
- **Typed video errors: `Error::JobFailed` and `Error::Decoding`.** `JobFailed {
  job, provider, id, status, message }` reports a video generation that the
  provider ended with a failure. `Decoding` reports a reply that lacks a field
  the result needs, such as a finished video operation with no video. Batch and
  transcription waits are unchanged: their `poll` already returns the failure
  cause.

### Changed

- **BREAKING — a `Message` is a role plus ordered `parts`.** Text, images,
  files, tool calls and tool results are parts of `Message.parts`. One
  assistant turn can carry text and tool calls together. `Message.content`,
  `Message.tool_calls` and `Message.tool_result` are removed; put the same
  content in `Message.parts`. `Message::new(role, text)` now builds one text
  part, or none for empty text. `save_history` writes version 2, and
  `load_history` rejects a version 1 document.
- **BREAKING — `Response::text` is a method, and `Response.parts` holds the
  reply.** `parts` holds the assistant turn as ordered text and tool-call
  parts. `resp.text()` joins the text parts, and `resp.tool_calls()`
  returns the tool-call parts. Replace `resp.text` with `resp.text()`.
- **BREAKING — a failed Agent call leaves the history unchanged.** Once an
  Agent call has started, an error restores the history to what it was
  before the call. Before, the history kept the user turn, assistant turns
  and tool results of the failed call. The new `Agent::unsaved_turns()`
  returns those turns, in order. It is empty after a call that succeeded or
  was rejected before it started. The call returns the error that stopped
  it, as before.
- **BREAKING — `VideoHandle` and `VideoRequest` gain public fields.**
  `VideoHandle.keep_stored` records the `keep_stored()` choice. A struct
  literal of `VideoHandle` must set it; `false` is the default.
  `VideoRequest` gains `last_frame`, `reference_images`,
  `duration_seconds`, `aspect_ratio` and `resolution`. A struct literal of
  `VideoRequest` must set them, or end with `..Default::default()`.
- **BREAKING — a rejected request for a listed model names the facts it
  contradicts.** `Error::Api` gains `hint: Option<String>`. When the
  provider answers 400 or 422, it names the stated facts the request
  contradicts, for example `catalogue: heygen/heygen-video-1 lists no last
  frame, at most 15 seconds`. It covers image input, aspect ratio and image
  size on image; image-to-video, last frame, reference images, maximum
  duration, resolution and aspect ratio on video; and speech style on
  speech. `Display` appends `; ` and the hint. The hint is `None` for a
  model the SDK does not list, for a fact no source states, and for any
  other status. The `Api` variant is now `#[non_exhaustive]`: match it with
  `..`, and only the crate builds it, so later fields are not breaking.
  Middleware post events get the same error text, hint included.
- **BREAKING — `Text::stream` returns a `TextStream`.** It no longer takes a
  callback and is no longer `async`. Pull each delta with
  `stream.next().await`, which returns `Option<Result<String, Error>>`; after
  it returns `None`, `stream.response()` holds the final `Response`. The
  request is sent on the first `next()` and never again (a first `next()`
  dropped before it returns ends the stream), an error is the last item, and
  dropping the stream cancels the request. The stream is `Send` on native
  targets. `TextStream` is exported from the crate root. Migration:

  ```rust
  // before
  let resp = c.text().stream("Tell me a joke", |chunk| print!("{chunk}")).await?;
  // after
  let mut stream = c.text().stream("Tell me a joke");
  while let Some(chunk) = stream.next().await {
      print!("{}", chunk?);
  }
  let resp = stream.response().expect("the stream ended");
  ```

- **BREAKING — `Event` gains `model_unlisted`, which marks a model the SDK
  does not list.** Image, speech, music, video, chat, agent and batch set it
  on their pre and post middleware events. It is true when the SDK's model
  list does not hold the model for that provider, so no per-model check ran
  before the request was sent. It is not a telemetry attribute. `Event` has
  only public fields, so a caller that builds it with every field named, and
  no `..Default::default()`, no longer compiles.

- **BREAKING — `Models::list()` lists models live, and the compiled-in list
  is gone.** `c.models().list().await` calls each configured provider's
  models endpoint and returns a `LiveResult`: the models from every provider
  that answered, plus a per-provider `errors` map. It takes the place of
  `Models::live()`, which is removed. The synchronous `Models::list()` and
  `Models::get(id)`, which read a list compiled into the SDK, are removed.
  Use `c.models().provider(p).get(id).await` to look up one model. Vertex
  and Bedrock still report a `ProviderError` with `kind`
  `"not_supported"` in `LiveResult.errors`. `Models` and `ScopedModels` gain a private field,
  so a caller can no longer build either with a struct literal; use
  `c.models()` and `provider(p)`.
- **BREAKING — `llmkit::catalogue` and `llmkit::models_parsers` are no
  longer public.** The model list compiled into the SDK
  (`COMPILED_IN_MODELS`), the live-endpoint tables and the `/models`
  response parsers are internal. Use `c.models().list().await` for the live
  list, or `c.models().provider(p).get(id).await` for one model.
- **Each client caches the live model list for one hour.** A repeat
  `models().list()` or `provider(p).list()` within the hour makes no HTTP
  call, and `provider(p).get(id)` answers from the cached list when it holds
  the id. A failed call is not cached. Each list is kept per provider, base URL
  and API key, so a call sent to another endpoint or with another key
  makes its own request. Clones of a `Client` share the cache.
  The new `Models::fresh()` makes the call again and refills the cache; it
  carries through `provider(p)`. Sends never read the cache. A list served
  from the cache fires no middleware event.

- **BREAKING — a model that states it lacks a feature no longer blocks the
  request by itself.** A seed image, a last frame, reference images, image
  parts or a speech style sent to such a listed model now reaches the
  provider, which answers. The SDK still rejects it before sending where the
  provider ignores the field without an error, such as a speech style on
  OpenRouter. Where the provider's request has no field for the input,
  `Error::Validation` says so, for example `zhipu has no seed frame
  request`. A request with two such inputs reads `grok has no seed frame and
  last frame request`. The speaker-count and image-count limits are
  unchanged.

- **BREAKING — every request times out after 600 seconds without data.**
  The client waits at most 600 seconds for the response headers, and for
  each next chunk of the body. A long stream that keeps sending does not time
  out. Before, a request waited with no limit. The new
  `Client::timeout(d)` sets another limit, and `Duration::ZERO` turns it
  off. On expiry every target cancels the request and returns the new
  `Error::Timeout`. A timeout from the operating system, such as a TCP
  connect the kernel gave up on, stays `Error::Http`. `Provider` and
  `ProviderConfig` gain a public `timeout` field, and `Provider` is now
  `#[non_exhaustive]`. Build a `Provider` with `Provider::new(name, key)`.
- **BREAKING — `decode_request` reads an integer option's decimal text
  exactly.** `max_tokens`, `seed` and `top_k` are read from the body's text
  as digits times a power of ten, never as an `f64`. `9007199254740993.0` and
  `9.007199254740993e15` decode to `9007199254740993`; before, they decoded
  to `9007199254740994`. A fraction such as `9007199254740991.5` is an
  `Error::Validation`; before, it rounded to a whole number and was accepted.
  A value outside i64 is still an `Error::Validation`, and `1e3` and `5.0`
  still decode to `1000` and `5`.

- **BREAKING — `Response::finish_reason` is an `Option<FinishReason>`.**
  `FinishReason` has five variants shared by every provider: `Stop`,
  `Length`, `ToolCalls`, `ContentFilter` and `Other`, with `as_str()` for the
  string; `None` means the provider reported none. Anthropic `end_turn` is
  `Stop`, Anthropic `refusal` and Google `SAFETY`, `RECITATION`, `BLOCKLIST`,
  `PROHIBITED_CONTENT` and `SPII` are `ContentFilter`, and a Responses
  `incomplete` is `Length` or `ContentFilter` by its
  `incomplete_details.reason`. A value with no canonical meaning is `Other`.
  The provider's own string moved to `Response::provider_finish_reason`
  (`Option<String>`). Compare
  `resp.finish_reason == Some(FinishReason::Length)`, or read
  `resp.provider_finish_reason`. `FinishReason` is exported from the crate
  root.
- **BREAKING — `encode_response` writes the target format's own finish
  reason.** It keeps `provider_finish_reason` when the target format has that
  value with the same meaning, so a same-format round trip is exact, and
  otherwise writes that format's value for the canonical reason: an Anthropic
  `end_turn` encoded for OpenAI is `stop`. A reason the target format cannot
  express is written as that format's fallback value: `Other` is `stop` for
  OpenAI, `end_turn` for Anthropic and Bedrock, `OTHER` for Google and
  `completed` for Responses; `ToolCalls` is `STOP` for Google and `completed`
  for Responses; `ContentFilter` is `end_turn` for Bedrock.
- **BREAKING — `Response::chat_wire_shape` (`Option<String>`) names the
  format a reply was decoded from,** such as `"ChatAnthropic"`. One-shot,
  stream, batch and agent replies set it; it is `None` on a `Response` you
  build yourself. A struct literal that lists every field must add it, or end
  with `..Default::default()`. When it equals the target format,
  `encode_response` writes `provider_finish_reason` verbatim even when the
  value has no canonical meaning: an Anthropic `pause_turn` decodes as
  `Other` and encodes back as `pause_turn`.
- **BREAKING — `encode_response` rejects a format the provider does not
  serve.** It returns `Error::Validation` with field `"chat_wire_shape"`, for example
  for Vertex, which serves no chat format.
  `encode_response(ProviderName::OpenAI, "ChatOpenAI", &resp)` is unchanged.
- **`encode_response` for OpenAI Chat Completions writes `object`
  (`"chat.completion"`), `choices[0].index` (`0`), `choices[0].message.role`
  (`"assistant"`) and, when the response has one, `created`.**
- **Bedrock replies now report a finish reason.** `finish_reason` and
  `provider_finish_reason` are read from `stopReason`, and `encode_response`
  writes it.
- **Streaming and one-shot sends apply caching with the selected wire
  format's config,** the same config they encode with.
- **The Vertex and Gemini music replies read the filtered reason at one
  declared path.** Vertex reads `predictions[0].raiFilteredReason`.

- A tool call's arguments go out as their text in Anthropic, Google and
  Bedrock request bodies, in `encode_response` and in `save_history`, without
  the spaces between tokens: `{"city": "Oslo"}` is sent as `{"city":"Oslo"}`.
  The next agent request replays the provider's own turn with its arguments
  as the provider wrote them; it used to sort their keys. OpenAI's string
  arguments keep their spaces.
- A tool call with `input` `None`, `"null"` or `""` sends `{}` in every
  request. `""` used to fail with `Error::Validation`.
- **BREAKING — a provider's `"input": ""` is an argument value.** In an
  Anthropic, Google or Bedrock reply, the call's `input` is `Some("\"\"")`,
  and the next request sends it. It used to decode as `None`. Only `null`
  reads as `None`.
- **BREAKING — a tool call input must be JSON as written.** An input that
  is JSON only after its whitespace is removed, such as `"1 2"` or `{}`
  with a form feed inside, fails `save_history` and every request. It used
  to pass as `12` or `{}`.
- **BREAKING — U+2028 and U+2029 are written as `\u2028` and `\u2029`** in every key
  and string the SDK writes: prompt text, OpenAI's string tool-call
  arguments, the image edit form's extra fields and the telemetry export.
  A provider's own text, such as a tool call's arguments or a replayed
  turn, is escaped too: it keeps the same JSON value. These are the bytes
  the Go SDK sends. They used to go out as the characters.
- **BREAKING — a saved provider turn must be one JSON value of its format's
  kind.** `load_history` and every request (`text()`, stream, batch, the
  Agent, `encode_request`) return a validation error on
  `messages[i].provider_turn` when the turn's text is not JSON, or is an
  object where the format replays an array (Anthropic, Responses) or an array
  where it replays an object (OpenAI Chat Completions, Google). Such a turn
  used to be rebuilt from the message's parts, or sent as it was.
- **BREAKING — an empty text part is a validation error on every request
  and in `save_history`,** naming the part, for example `parts[0]`: chat,
  image, music, video and transcription. A chat request used to send an
  empty Anthropic text block, the others sent the part, and `save_history`
  saved it. Speech takes no parts.
- **BREAKING — `WireError` has a `Validation { field, message }` variant.**
  `load_history` returns it for a bad provider turn, and `save_history` for
  a tool call whose input is not JSON, the empty text included, on
  `messages[i].parts[j]`. `save_history` used to return
  `WireError::Malformed` there. An exhaustive `match` on `WireError` needs
  the new arm.
- **BREAKING — an empty `Lyrics` or `AudioUrl` part is a validation error**
  with "empty part", on `parts[i]` at request and on `messages[i].parts[j]`
  in `save_history`, as in Go. Both used to be accepted.

- **BREAKING — a mask is accepted only on an OpenAI request with reference images.** A
  mask on any other image request is rejected before the request is sent.

- **BREAKING — image input limits are per model, and the provider-level count is gone.**
  The provider-level count mixed two kinds of number: wire limits, and
  provider maxima (Google 14, OpenAI and Grok 16) that no source confirmed.
  When a provider raised its limit, image generation rejected calls the
  provider would accept. Image generation now rejects image parts for a
  model that takes none (OpenRouter, Recraft and Vertex models). It rejects
  more images than the model's limit when the catalogue states one. Every
  other model sends all image parts and lets the provider decide.
  Migration: read `ImageHints::image_input` and
  `ImageHints::max_input_images` from `ModelInfo.image` instead of the
  provider-level count.

- **Text parts join with a newline.** The text builder joins parts added with
  `.text(...)` and the prompt with a newline. It used to join them with nothing,
  so the request no longer depends on which SDK sent it.

- **BREAKING — an unreported usage dimension or finish signal is now absent, not zero.**
  The six `Usage` dimensions (input, output, cache read, cache write, reasoning,
  cost) and the finish reason / finish message on every response container are
  now `Option<i64>` / `Option<f64>` / `Option<String>`. Previously a provider that reported no
  cached tokens and one that never mentioned caching both produced `0`, and an
  unreported cost read as a free request.

  Migration: `resp.usage.input` is now `Option<i64>`; match or `if let` rather than treating a missing count as `0`.

  Aggregation across agent turns is **absorbing** — a total is reported only when
  every turn reported that dimension, so a partial sum is never presented as a
  total. Telemetry now exports a usage attribute when the value is *reported*,
  zero included, instead of when it is greater than zero.

- **BREAKING — `wait_video` returns typed errors.** A job the provider
  failed returns `Error::JobFailed`, a done job with no readable video returns
  `Error::Decoding`, and the deadline returns `Error::PollTimeout`, as the
  batch and transcription waits already did. All three used to be
  `Error::Unsupported`. A failed status check is still `Error::Api` with its
  HTTP status code. Code that matched `Error::Unsupported` for these cases must
  match the new variants. The error text changes:
  - Every message loses its "unsupported: " prefix. A `Decoding` message
    starts with "decoding: " instead.
  - The deadline reads "poll: deadline exceeded for <provider> job <id>; ..."
    instead of "video poll: timed out waiting for <id>".
  - A failure with a provider message reads "video generation failed:
    <message>", as before. Grok's "expired" status now reads "failed" too.
  - A status-only failure reads "video generation failed: <status>", with the
    provider's raw status. It used to vary by provider: "video generation
    cancelled" (Together), "video generation FAILED" (Qwen), "video
    generation failed (status 8)" (PixVerse), and "video generation failed"
    (Zhipu, MiniMax).
  - Vidu, Vertex Veo and Bedrock still read "video generation failed:
    operation failed" when the provider sends no message.

### Removed

- **BREAKING — Vertex image generation removed.** Google shut down the Imagen endpoints.
  Image generation on Vertex now fails with a validation error before any
  request is sent. Vertex music and video are unchanged, and the Vertex default
  model is now `lyria-002`.
- **BREAKING — the image safety filter is removed.** `Image::safety_filter`
  and the `IMAGE_SAFETY_FILTER_BLOCK_*` constants are gone. No provider
  accepted a safety threshold, so every call returned `Error::Validation`.
  Delete the call; `Image::safety_settings` stays.
- **BREAKING — `llmkit::providers` holds only the roster functions.** It
  keeps `info` and `list`. `ProviderName`, `UnknownProviderError`,
  `ProviderInfo` and `ALL_PROVIDER_NAMES` have one path each, at the crate
  root: write `llmkit::ProviderName`, not `llmkit::providers::ProviderName`.
  `llmkit::providers::generated` is gone, with the config
  lookups (`image_gen_config`, `video_gen_config`, `speech_gen_config`,
  `music_gen_config`) and every other table. Use the crate-root names, such
  as `llmkit::ProviderName`, `llmkit::FinishReason` and `llmkit::Event`.
  Read model facts from the `ModelInfo` hints:
  `let info = c.models().provider(p).get("gpt-image-1").await?;`, then
  `info.image.and_then(|i| i.max_input_images)`. In place of the
  caching table, ask `c.supports(Capability::Caching)`.
- **BREAKING — `llmkit::ImageOptions` and `llmkit::MusicOptions` are no
  longer public.** No public function took either type. Set the same values
  on `client.image()` and `client.music()`.
- **BREAKING — Google video no longer runs the Veo preview models.** Google
  shut down the Gemini API Veo preview models, such as
  `veo-3.1-generate-preview`. A 2.x call that names one now fails. The
  `google` provider now serves `gemini-omni-1.1-flash` on the Interactions API
  (see Added). It takes text only. The SDK rejects a seed image before send,
  as it did for Veo. Veo stays available on Vertex.
- **BREAKING — `llmkit::ImageRequest` and `llmkit::MusicRequest` are no
  longer public.** No public function took either type. Build each request
  on `client.image()` and `client.music()`. `VideoRequest` stays, for
  `submit_video`.
- **BREAKING — `llmkit::sigv4` and `llmkit::set_event_error` are no longer
  public.** Both were hidden from the docs and served only the repository's
  own tests. Bedrock requests are still signed.
- **BREAKING — `llmkit::builders::DEFAULT_TIMEOUT`,
  `llmkit::builders::AgentState` and
  `llmkit::telemetry::build_telemetry_payload_at` are no longer public.** No
  public function takes or returns them. A client still waits 600 seconds
  by default. `AgentState` was hidden from the docs.
- **BREAKING — `llmkit::builders::catalogue` is private, and the catalogue
  builders' fields are crate-internal.** Import `Models`, `Providers` and
  `ScopedModels` from `llmkit::builders`. The fields `client`, `target`,
  `cap_filter` and `raw_flag` held builder state. Set them with the chain
  methods `with_capability(c)`, `provider(p)` and `raw()`.
- **BREAKING — `ImageData`, `MediaRef`, `wait_transcription`,
  `TranscriptionPoll` and `CatalogueError` have one path, the crate root.**
  The copies in `llmkit::builders` are gone, and `llmkit::models` is no
  longer public. Import them as `llmkit::ImageData` and so on.

### Fixed

- `Agent::prompt` leaves the history unchanged when it rejects the request,
  for example an option the provider does not take. The rejected user turn
  used to stay in the history, so the next prompt sent two user turns.

- **OpenAI batch failures show the provider's error.** Waiting on an OpenAI
  batch now also reads its error file. A failed request's `cause.status` is the
  provider's error code (for example `batch_expired`), and its `cause.message`
  is the provider's error message.
- **A batch result succeeds only when the provider says it did.** A result line
  with a failure status and an empty body reads as failed, not as a success
  with empty text.
- **A batch returns one response per prompt, even when the last one has no
  result.** The list length comes from the batch's request count, so a missing
  last request keeps its slot, with `cause.code` `JobFailureCode::Missing`.
- **Batch results are matched to prompts by request id, not by line order.**
  Providers return batch results in any order, and a failed request used to be
  dropped, so later answers moved up one place. Waiting on a batch now returns
  one response per prompt, at the prompt's index. A failed request keeps its
  slot with empty text, `state` `Some(JobState::Failed)`, the provider's result status in
  `cause.status` (`errored`, `expired`, `canceled`, or `error`) and the provider's
  error message in `cause.message`. A request with no result has `cause.code`
  `JobFailureCode::Missing`. Results for request ids this SDK did not assign follow in file
  order.

- **Anthropic streams report the finish reason.** It was read from the
  `message_stop` event, which never carries it, so it was always null. It is
  now read from `message_delta`.
- **Parallel tool results on AWS Bedrock.** After a reply with several tool
  calls, the agent sent one user message per result, and Bedrock rejected the
  request with 400. The results now go in one user message, in call order.

## [2.2.2] — 2026-10-27

### Fixed
- `text().history(...).prompt(text)` now sends the images and files attached
  to the prompt on its last user turn, after the history. Before, they were
  dropped when a history was set.

## [2.2.1] — 2026-10-07

### Changed
- The crate ships only its source, manifests, `README.md`, `CHANGELOG.md` and `LICENSE`.
- Doc comments ship again, so docs.rs shows descriptions.
- The error for a chat protocol on a terminal other than prompt no longer ends with an internal reference.

### Removed
- 2.2.0 is no longer available on crates.io. Use 2.2.1.

## [2.2.0] — 2026-10-04

### Added

- `decode_response(provider, chat_wire_shape, body)` turns a provider's chat response body into a `Response`, the same way the client reads it. `encode_response(provider, chat_wire_shape, &response)` writes a `Response` back into that provider's body shape. Both are pure functions: no client, no key, no network. `encode_response` returns an error for a field it cannot write back faithfully.

### Fixed

- Middleware now runs for speech generation and transcription requests, as the new `speech_generation` and `transcription` operations, and its pre phase can veto them. Before, these requests ran no middleware, so a policy hook could not block them. For transcription it runs on the sync and async calls, not on waits or polls.
- OpenAI Responses protocol: gpt-5 and o-series models now send the output-token cap as `max_output_tokens`. They sent `max_completion_tokens`, which the Responses API rejects with HTTP 400.
- Anthropic requests no longer send `anthropic-beta: files-api-2025-04-14`. The Files API left beta, and with that header an upload returned the older response shape, without `expires_at`.

## [2.1.0] — 2026-07-20

### Added

- Every response and catalogue type is now re-exported from the crate root uniformly: `SpeechResponse`, `TranscriptionResponse`, `TranscriptSegment`, and `TranscriptionHandle` join the existing root re-exports (`VideoResponse`, `ModelInfo`, `LiveResult`, and the rest). Previously a consumer naming a speech or transcription result had to import from the internal module path.

### Changed

- Published on crates.io as `llmkit-rust` (the `llmkit` crate name is held by an unrelated project). The library target keeps the name `llmkit`, so imports are unchanged: `cargo add llmkit-rust`, then `use llmkit::...` as before.

## [2.0.1] — 2026-07-19

### Security

- Fixed: an API key could appear in error output. On a failed request to a query-parameter-authenticated provider (Google), the transport error's `Display` embedded the full request URL — which carries the key as a `?key=` query parameter — so logging the error could leak the key. The URL is now stripped from transport errors.

## [2.0.0] — 2026-07-19

### Breaking

- Clean async-job API. Batch is now a single async terminal on the `text` builder — `c.text().<chain>.batch(...).await` returns a `BatchHandle` (batch is a text execution mode, parallel to `stream`), and `handle.wait().await` resolves the ordered results. The old two-terminal surface is collapsed: the blocking `batch` (which returned `Vec<Response>`) and `submit_batch` are both gone — `batch` now returns the handle. The handle also `impl IntoFuture`, so `c.text().<chain>.batch(...).await?.await?` works. Migration: `c.text().<chain>.submit_batch(...)` → `c.text().<chain>.batch(...)`; the old blocking form → `let h = c.text().<chain>.batch(...).await?; h.wait().await?`.

### Added

- Typed telemetry error kind. The middleware `Event` carries a typed `err_type` set structurally from the error, and the OTLP span's `error.type` attribute now derives from it rather than from string classification of the message. Additive.

### Fixed

- Streamed OpenAI usage is no longer `0`: the SDK opts into `stream_options.include_usage` per provider (OpenAI), so streamed calls report real input/output token counts.
- A batch with an errored or unparseable result line now returns the successful subset instead of discarding the whole batch.
- Image and file input Parts are carried through the batch request envelope.
- The `models_list` middleware op now fires real client hooks.
- `with_capability(...)` now filters the scoped provider list.
- A malformed 2xx speech-generation body is now a typed decoding error instead of silent empty audio.
- The per-request `anthropic-beta` header is sent on batch submit, so a file-referencing batch item no longer 400s.

## [1.2.1] — 2026-07-11

### Fixed

- `providers::info(ProviderName::OpenAI).browser_callable` is now `false` (was `true`). OpenAI's API host answers the CORS preflight but omits `Access-Control-Allow-Origin` on the actual response, so a browser cannot read it directly. The flag now reflects the real response, not the preflight — a browser app that reads it to decide direct-call-vs-proxy will correctly route OpenAI through a proxy. Google stays `true` (its responses carry the header). No API change.

## [1.2.0] — 2026-07-06

### Added

- Inline image input on the text/prompt path. `c.text().image(mime, bytes).prompt(...)` now sends the image as the provider's native vision block on all four chat wire shapes (Anthropic, OpenAI, Google, Bedrock). Bytes-based, so it works with no filesystem. Additive (the `.image(...)` builder method previously dropped the image on this path).

## [1.1.0] — 2026-06-09

### Added

- Video generation — `c.video().model(id).submit(prompt).await` returns a `VideoHandle`; `handle.wait().await` (or the free `wait_video(&handle, poll)`) polls until the job finishes and returns `VideoResponse { videos: Vec<VideoData>, usage, finish_reason, finish_message }`. Each `VideoData` carries `url`, `mime_type`, and `duration_seconds`. One provider so far: xAI Grok Imagine (`grok-imagine-video`), which delivers a temporary hosted URL — download it yourself.
- Music generation — `c.music().model(id).generate(prompt)` (or the free `generate_music(...)`) produces audio from a text prompt, with an optional `.lyrics(...)` chain method for models that support vocals. Returns `MusicResponse { audio: Vec<AudioData>, text, usage }` with decoded audio bytes. Three providers: Vertex Lyria 2 (`lyria-002`, instrumental WAV), Google Lyria 3 (`lyria-3-pro-preview` / `lyria-3-clip-preview`, MP3 with lyrics), and MiniMax (`music-2.6`). Instrumental-only models reject lyrics before the request is sent.
- `Response.finish_reason` and `Response.finish_message` — provider stop signal + free-text explanation passed through verbatim on `client.text().prompt()`, the agent loop, and `client.text().stream(msg, callback)` (carried on the returned `Response` once the callback shape resolves). Examples: Anthropic `stop_reason`, OpenAI `choices[0].finish_reason`, Google `candidates[0].finishReason`. Default empty `String`; populated only when the provider response carries a signal. Streaming uses an `event_name:json.path` locator — Anthropic captures from the `message_stop` event body; OpenAI/Grok/Google use last-non-empty-wins on the data frames; Google additionally filters `FINISH_REASON_UNSPECIFIED`. Bedrock Converse streaming is not yet wired.
- `ImageResponse.finish_reason` and `ImageResponse.finish_message` — same shape on `client.image().generate()`. Google populates both (including `IMAGE_OTHER` / `SAFETY` / `MAX_TOKENS` reasons that previously vanished into "no image returned"); Vertex Imagen surfaces `predictions[0].raiFilteredReason` as `finish_reason`; OpenAI Images API and xAI Grok have no equivalent fields and leave them empty.

## [1.0.0] — 2026-05-09

### Removed (Breaking)

- Legacy free-function layer deleted. The bodies live on as `pub(crate)` helpers consumed by the typed-builder terminals; the public surface is now exclusively the typed builder reachable via `llmkit::builders::new_client(...)`:

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

- Chain-field propagation lint integrated into `make check`.
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
- Multi-reference compositional generation now works by ordering the parts vec (e.g., `vec![Part::text("Person:"), Part::image(mime, ref_a), Part::text("Outfit:"), Part::image(mime, ref_b), Part::text("Generate ...")]`) — the wire shape preserves caller-controlled ordering.

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
