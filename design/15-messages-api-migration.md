# 15: Migration to the Messages API

Status: **Proposed**. Assumes [14-pdf-text.md](14-pdf-text.md) is implemented: PDFs reach
the model as extracted text, and there's no parse cache.

Every OpenRouter generation and title request moves from Chat Completions
(`/api/v1/chat/completions`) to OpenRouter's Anthropic-compatible Messages API
(`/api/v1/messages`). Chat Completions is no longer used. The model list
(`/api/v1/models`) and key status (`/api/v1/key`) stay as they are.

## Why

- **Bash needs it.** `openrouter:bash` ([16-openrouter-tools.md](16-openrouter-tools.md))
  only works on the Messages API. Keeping both APIs would mean two request builders, two
  stream parsers and two sets of fakes. Moving everything means one of each.
- **Nothing holds Sprinter on Chat Completions anymore.** The only feature that needed it
  was the PDF parse cache (`file` annotations), and that is removed by
  [14-pdf-text.md](14-pdf-text.md).
- **It works for every model.** OpenRouter translates Messages API requests for
  non-Anthropic models. The spike (below) covered Claude, GPT, Gemini, Grok, Llama, Kimi,
  DeepSeek, gpt-oss, MiniMax and Cohere.
- **Better stream metadata.** `message_start` reports the provider that actually served
  the request. The `provider` field on Chat Completions chunks was sometimes wrong.

## Decisions

| Area | Decision |
|---|---|
| Scope | Generation (streaming) and title (non-streaming) requests. Everything moves in one change, with no feature flag and no fallback to Chat Completions. |
| Client | Written by hand on `reqwest` and `serde_json::Value`, in `openrouter/client.rs`. No SDK crate (see [Why not a library](#why-not-a-library)). |
| `max_tokens` | **Not sent** on generation requests. Titles keep `max_tokens: 64`. |
| Reasoning | No request field. `thinking` and `redacted_thinking` blocks are dropped by the parser. |
| Sticky routing | Each chat gets a random `session_id`, sent on every request of that chat. |
| Unknown stream content | Ignored. The parser only acts on the event and block types it knows. |

## Request

```json
POST /api/v1/messages
{
  "model": "openai/gpt-6-luna",
  "stream": true,
  "session_id": "<chats.session_id>",
  "system": "<custom instructions>",
  "messages": [
    { "role": "user", "content": [
        { "type": "text", "text": "What's in this picture?" },
        { "type": "image", "source": { "type": "base64", "media_type": "image/webp", "data": "…" } } ] },
    { "role": "assistant", "content": "…" },
    { "role": "user", "content": "…" }
  ],
  "plugins": [ { "id": "context-compression", "enabled": false } ]
}
```

Mapping from the current Chat Completions request:

| Chat Completions | Messages API |
|---|---|
| `messages[0]` with `role: "system"` | Top-level `system` string. It's omitted when empty. |
| A user or assistant string | Unchanged |
| `{type: "text", text}` | Unchanged |
| `{type: "image_url", image_url: {url: "data:<mime>;base64,<data>"}}` | `{type: "image", source: {type: "base64", media_type, data}}` |
| `{type: "file", file: {filename, file_data}}` (only the scanned-PDF fallback from [14-pdf-text.md](14-pdf-text.md)) | `{type: "document", source: {type: "base64", media_type: "application/pdf", data}, title: filename}`, with `plugins` adding `file-parser` pinned to `native` |
| `reasoning: {exclude: true}` | Removed. The parser drops `thinking` blocks instead. |
| `plugins` | Unchanged: `context-compression` disabled, plus `file-parser` for the native fallback |
| `HTTP-Referer`, `X-Title` headers | Unchanged |
| (none) | `session_id` |

- **No `max_tokens`.** OpenRouter's Messages API doesn't require it, unlike Anthropic's
  own API. Sending a model's output limit would reject long chats on Claude, because the
  provider checks prompt plus `max_tokens` against the context window. In the spike, a
  180k-token prompt with `max_tokens: 64000` was rejected against a 200k context, and
  the same request without it went through.
- **Message order needs no changes.** Consecutive user messages, and assistant messages
  with empty content (left by a failed or cancelled reply), were accepted on Claude, GPT
  and Llama. Sprinter's root-to-leaf path is sent as it is.
- **`session_id`.** OpenRouter uses it as a sticky routing key, sending a session's
  requests to the same provider to improve prompt cache hits. It's a random 128-bit hex
  value, created with the chat and stored as `chats.session_id`. It's not the chat id,
  because the chat id is a UUIDv7 that reveals when the chat was created. The bash sandbox
  in [16-openrouter-tools.md](16-openrouter-tools.md) is keyed by the same value.
- **The `anthropic-version` header isn't sent.** OpenRouter doesn't need it.

### Titles

The title request uses the same endpoint without streaming, with `max_tokens: 64` and no
`session_id`. The title is the concatenated `text` blocks of `content`, trimmed.

## Stream parsing

The stream is SSE with named events. The parser reads `data:` JSON and uses its `type`
field:

| Event | Handling |
|---|---|
| `message_start` | Record `message.id` (`gen-…`, the generation id) and `message.provider`. The first-token log line uses this provider. |
| `content_block_start`, `text` block | Start collecting text for that block index. |
| `content_block_start`, `thinking` / `redacted_thinking` | Ignore the block and its deltas. |
| `content_block_start`, `server_tool_use` and tool result blocks | Passed on to the tool handling in [16-openrouter-tools.md](16-openrouter-tools.md). Until then, ignored. |
| `content_block_delta`, `text_delta` | `ProviderEvent::Delta`, as today |
| `content_block_delta`, `citations_delta` | Passed on to [16-openrouter-tools.md](16-openrouter-tools.md). Until then, ignored. |
| `content_block_delta`, other types (`thinking_delta`, `signature_delta`, `input_json_delta`) | Ignored, unless a tool handler claims the block |
| `content_block_stop` | Close the block. |
| `message_delta` | `stop_reason` and `usage`, kept for `Done` |
| `message_stop` | `ProviderEvent::Done`. **This is the completion marker.** |
| `ping` | Ignored |
| `error` | `ProviderError`, mapped as in [Errors](#errors) |
| `data: [DONE]` after `message_stop` (sent as `event: data`) | Ignored |
| Any other event or block type | Ignored |

- If the connection ends before `message_stop`, it's an error: "OpenRouter stream ended
  without a completion marker", as today with `[DONE]`.
- `ProviderEvent::Annotations` is removed. `file` annotations are gone after
  [14-pdf-text.md](14-pdf-text.md), and the Messages API has no `url_citation`
  annotations.

### Finish reason

`stop_reason` is stored in `messages.finish_reason` using Chat Completions' vocabulary, so
existing rows and the UI keep one set of values:

| `stop_reason` | Stored as |
|---|---|
| `end_turn`, `stop_sequence` | `stop` |
| `max_tokens` | `length` |
| `tool_use` | `tool_calls` |
| `refusal` | `content_filter` |
| Anything else | Stored as it is |

### Usage

Usage arrives in `message_delta.usage` and follows Anthropic's accounting. **`input_tokens`
doesn't include cached tokens.** A repeated 12.5k-token prompt reported `input_tokens: 3`
and `cache_read_input_tokens: 12510`.

| Sprinter field | Messages API source |
|---|---|
| `prompt_tokens` | `input_tokens + cache_read_input_tokens + cache_creation_input_tokens`, treating nulls as 0 |
| `completion_tokens` | `output_tokens` |
| `reasoning_tokens` | `output_tokens_details.thinking_tokens` |
| `cost` | `cost`, still optional |

Without the sum, the spend view's token counts would drop sharply on every cached turn.
Cost is unaffected.

## Errors

Errors use Anthropic's shape, with no numeric `code`:

```json
{ "type": "error",
  "error": { "type": "invalid_request_error", "message": "…", "error_type": "invalid_request" },
  "request_id": "gen-…" }
```

The mapping uses the HTTP status first, then `error.error_type` (the only source of
information for mid-stream `event: error`):

| Status / `error_type` | Message shown |
|---|---|
| 401, `authentication` | "OpenRouter rejected the saved API key. Check Settings." |
| 402, `payment_required` | "OpenRouter has insufficient credits for this request." |
| 429, `rate_limit_exceeded` | "OpenRouter rate limited this request. Try again shortly." |
| 400 with "maximum context length" in `message` | The existing "too long for the selected model" text |
| 5xx | "OpenRouter is temporarily unavailable." |
| Anything else | `error.message` when there is one, otherwise "OpenRouter rejected this request." |

The context-length check matches on the message text, because `error_type` is just
`invalid_request`. It falls through to the upstream message, which is readable anyway
("This endpoint's maximum context length is 200000 tokens…").

## Why not a library

No crate made this simpler (checked 2026-09-29):
- There's no official Anthropic Rust SDK. Community Anthropic crates don't know about
  OpenRouter's `plugins`, `provider`, `session_id` or `openrouter:*` tools.
- `openrouter-rs` 0.16 does support OpenRouter's Messages API, but:
  - It parses content blocks into a fixed enum with no catch-all, so a new OpenRouter block
    type would make a whole reply fail.
  - It builds a new HTTP client on every request, so there's no connection reuse and no
    connect timeout.
  - It pulls in `tokio` with `full`, `schemars`, `derive_builder` and `dotenvy_macro`.
- The current client already parses SSE into `serde_json::Value` in about 250 lines. The
  Messages version is about the same size.

## Code changes

- **`openrouter/client.rs`:** new request builder, stream parser, usage and error
  mapping, and titles. The Chat Completions code is deleted.
- **`uploads.rs`:** content parts use the Messages block shapes above.
- **`generation.rs`:** creates `chats.session_id` if it's missing, drops the annotation
  handling, and maps `finish_reason`.
- **`fake-openrouter`:** serves `/api/v1/messages`, streaming and not, in the recorded
  event format. That includes the `event: data` / `[DONE]` tail, a `thinking` block, an
  unknown block type, `cache_read_input_tokens` in usage, and Anthropic-shaped errors.
  Existing triggers (`[[error]]` and the others) keep working. `/chat/completions` is
  removed.

## Data model changes

```sql
ALTER TABLE chats ADD COLUMN session_id TEXT;   -- random 128-bit hex, created on first generation
```

Existing chats get a `session_id` on their next generation.

## Logging

- The first-token line takes `provider` from `message_start`.
- `or_id` is `message.id`.
- The completed line adds `cache_read_tokens`, so the prompt-token sum can be checked.
- `session_id` is logged at `DEBUG` only.

## Testing

- **Backend:**
  - The request body for each content type, including the `document` fallback and no
    `max_tokens`.
  - Parsing each event type, including an unknown block, a `thinking` block that must not
    reach the content, the `[DONE]` tail, and a stream cut off before `message_stop`.
  - Usage summing and the `stop_reason` mapping.
  - Error mapping for the HTTP statuses and a mid-stream `event: error`.
  - Title parsing.
- **E2E:** unchanged scenarios against the new fake, with request-body assertions updated
  to the Messages shape.

## Risks

- **A translation layer.** For non-Anthropic models, OpenRouter converts the request.
  Chat Completions is OpenRouter's main API, so new provider-specific options may appear
  there first or only there. The Messages schema has no `reasoning` object.
  Reasoning-effort settings (deferred in [07-settings.md](07-settings.md)) would go
  through Anthropic's `thinking` / `output_config`, mapped by OpenRouter to other
  providers. That mapping is untested.
- **Mid-stream errors** weren't observed in the spike. The parser follows Anthropic's
  documented `event: error` shape, and the fake reproduces it.

## Open questions

1. **Reasoning effort.** When per-chat reasoning settings are designed, check how
   `thinking` / `output_config.effort` map to OpenAI and Gemini reasoning levels through
   OpenRouter.
2. **`stop_reason` from non-Anthropic models.** Only `end_turn` was observed. Check what
   OpenRouter reports for a length cut-off or a content filter on GPT and Gemini.

## Spike (2026-09-29)

These tests ran against the live API. The account enforces ZDR for the OpenAI and
Anthropic model groups.

| Test | Result |
|---|---|
| `claude-haiku-4.5` and `gpt-6-luna`, a PDF `document` block with `file-parser` and `context-compression` disabled | Accepted, and the PDF was read. No `file` annotation came back, streaming or not. |
| A base64 `image` block, both models | Accepted, with the same prompt token count as Chat Completions |
| Llama 4, Gemini 3.5, Grok 4.7, Kimi K3, DeepSeek V4.1, gpt-oss, MiniMax M3, Cohere Command A | All answered. Qwen and GLM were excluded by the account's ZDR settings, as they are on Chat Completions. |
| Bad model, bad key, context too long | 400, 401 and 400 in Anthropic's error shape, with a readable `message` |
| No `max_tokens`, short prompts, Claude and GPT | Accepted |
| `max_tokens: 1000000` | Llama, DeepSeek and GPT accepted it. Claude rejected it as prompt + output > context. |
| A 180k-token prompt on Claude Haiku (200k context) | `max_tokens: 64000` was rejected. Without `max_tokens` it was accepted, same as Chat Completions. |
| Consecutive user messages, and an empty assistant message | Accepted on Claude, GPT and Llama |
| A repeated 12.5k-token prompt on `gpt-6-luna` | `input_tokens: 3`, `cache_read_input_tokens: 12510`. Chat Completions reported `prompt_tokens: 12513` with `cached_tokens: 12510`. |
| Stream framing | `message_start` has `provider` (`Azure`), `model` and the `gen-…` id. The stream ends with `message_stop`, then `event: data` / `data: [DONE]`. |
| Streaming `claude-haiku-4.5` and `gemini-3.5-flash` | `thinking` and `redacted_thinking` blocks appear alongside `text` |

Sources: [Messages API reference](https://openrouter.ai/docs/api/api-reference/anthropic-messages/create-a-message.md),
[openrouter-rs](https://crates.io/crates/openrouter-rs), and
[Anthropic client SDKs](https://docs.anthropic.com/en/api/client-sdks).
