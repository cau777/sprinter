# 01: Chat model and generation

Status: **Decided.** The schema is in [09-data-model.md](09-data-model.md) and the endpoints are in [10-api.md](10-api.md).

## v1 feature set

| Feature | In v1 | Notes |
|---|---|---|
| Send and stream replies | Yes | |
| Chat CRUD (create, list, open, rename, delete) | Yes | |
| Model picker per chat | Yes | Default model is set in Settings. The model can change mid-chat, and each assistant message records which model produced it. |
| Edit user message and regenerate reply, with branches | Yes | ChatGPT-style `< 2/3 >` navigation between sibling branches. |
| Full-text search across chats | Yes | |
| Auto-generated chat titles | Yes | Titles come from a cheap model after the first exchange. The title model is configurable in Settings. |
| System prompts / presets | **No** (deferred) | The schema should not block adding them later. |

## Conversation structure: a message tree

A chat is a **tree** of messages, not a list. This is what makes edits and regenerations
non-destructive.

- Each message has a `parent_id`. The root user message has a null parent.
- **Editing** a user message creates a new sibling user message, which gets its own reply.
- **Regenerating** a reply creates a new sibling assistant message under the same parent.
- The chat stores `current_leaf_id`. The visible thread is the path from the root to that
  leaf.
- Switching branches moves `current_leaf_id` to the most recent leaf in the chosen
  sibling's subtree.
- The prompt sent to the model is always that root-to-leaf path.

## Generation lifecycle: owned by the server

The server owns each generation, not the HTTP request. If the client disconnects (tab
closed, phone locked), the generation keeps running and its result is persisted.

1. `POST /api/chats/:id/messages` (also used for edits) or `POST /api/messages/:id/regenerate` validates the request and
   inserts the user message plus an empty assistant message with `status = streaming`. It
   spawns a background task and returns the new message IDs right away.
2. The background task calls OpenRouter with `stream: true` and appends deltas to an
   in-memory buffer, which fans out to subscribers. It flushes the accumulated content to
   the DB periodically (for example every ~1 s) and on completion.
3. The client subscribes to `GET /api/messages/:id/stream`, which is SSE. A subscriber first
   gets everything generated so far, then live deltas. Reconnecting after a drop gives the
   same result, so reattaching is seamless.
4. On completion the message gets `status = complete` and records usage and cost data from
   OpenRouter, finish reason, and model. On failure it gets `status = error` with the
   message.
5. The user can **stop** a generation explicitly with `POST /api/messages/:id/cancel`.
   This and deleting the chat are the only things that cancel it. The partial content is kept and the status becomes
   `cancelled`.
6. On server start, any message still in `streaming` status (left over from a crash or
   restart) is marked `interrupted`.
7. **Graceful shutdown (SIGTERM):** the server stops accepting requests and gives
   in-flight generations up to 8 s to finish, which fits inside Docker's default 10 s
   stop timeout. It then flushes partial content and marks those messages `interrupted`.

### Concurrency rules

- **One active generation per chat.** Sending or regenerating while that chat has a
  `streaming` message returns `409 generation_in_progress`. The UI prevents this anyway,
  because the Send button becomes Stop.
- Generations in different chats run in parallel, up to a cap of 8 in total, after which
  the server returns `429`.
- Deleting a chat cancels its active generation first.

### Failure handling

- **No API key set:** sending returns `409 no_api_key`, and the UI links to Settings.
- **Provider errors** (401 bad key, 402 no credits, 429 rate limited, 5xx, a network
  drop mid-stream) set the message to `error` with a readable message and keep any
  partial content. There are **no automatic retries**. The user clicks Retry, which is
  the same as regenerate.
- **Context too long:** there's no silent truncation, and OpenRouter's context compression
  plugin is explicitly disabled (`plugins: [{id: "context-compression", enabled: false}]`).
  The error says the conversation is too long for the model and suggests switching to a
  model with a larger context or starting a new chat.

### Outgoing request shape

- `model`: the chat's current model, or the model picked in the regenerate menu.
- `messages`: the global custom instructions (if any) as `system`, then the root-to-leaf
  path, with attachments expanded as described in [02-files.md](02-files.md).
- `stream: true` and `reasoning: { exclude: true }`. OpenRouter always returns usage;
  the streaming final chunk includes it alongside a final choice and `finish_reason`.
- The PDF `file-parser` plugin with a request-level engine setting (see [02-files.md](02-files.md)).
- Headers `HTTP-Referer: https://github.com/cau777/sprinter` and `X-Title: Sprinter`,
  for OpenRouter app attribution.
- **Model can't see images:** if the target model lacks image input but the path
  contains images, each image is replaced by the text `[image omitted: <filename>. The
  current model can't view images]`, and the UI shows a one-line warning above the
  composer. PDFs and text work with any model.

SSE is chosen over WebSockets. The traffic is one-directional (control actions are plain
POSTs), SSE works through every proxy, and `EventSource`/fetch-stream reconnection is
straightforward.

## Reasoning (thinking) output

Hidden. Requests send OpenRouter's `reasoning: { exclude: true }`, so reasoning text is
never streamed, stored, or shown. Reasoning models still think, and are billed for it:
`reasoning_tokens` and cost are recorded. Until the first content token arrives, the
reply shows the streaming glow with a muted "Thinking…" label, so a long reasoning phase
doesn't look stalled.

## Titles

When the **first user message** of a chat is sent, a background task asks the title
model for a short title, based only on that message. This runs in parallel with the
reply, so the title is usually ready before the reply finishes. It reaches the client
as an `event: title` on that reply's SSE stream. If the title isn't ready when the reply
finishes, that first stream stays open after `done` until the `title` event is sent
(10 s at most). A client that missed it still sees the title on its next chat-list
fetch.

- **Title model:** the one set in Settings. If none is set, the chat's model is used.
- If title generation fails, the title falls back to the first ~60 characters of the
  message.
- The user can rename the chat at any time. A manual rename is never overwritten.

## Search

Search uses SQLite FTS5 over message content and chat titles. Results link to the chat, and to the matching message where
possible.

## OpenRouter protocol spike (2026-09-25)

Checked the current official documentation index and API reference at
`https://openrouter.ai/docs/llms.txt` and its linked Markdown pages, plus the live
`GET https://openrouter.ai/api/v1/models` endpoint. The live endpoint returns
`{"data":[...]}` and model records expose `id`, `name`, `context_length`, `pricing`, and
`architecture.input_modalities` / `output_modalities`.

- Usage is always returned. The current API marks `stream_options.include_usage` as
  deprecated with no effect; `usage: {include: true}` is not the current request shape.
  The streaming final chunk contains a content-free choice repeating `finish_reason`
  and a `usage` object. `usage.cost` is optional, so the client must handle its absence.
- `GET /api/v1/credits` returns `data.total_credits` and `data.total_usage`, but now
  requires a Management API key. `GET /api/v1/key` uses the regular API key and returns
  `data.limit`, `limit_remaining`, `limit_reset`, and usage fields. A direct unauthenticated
  `/credits` call returned HTTP 401.
- `reasoning: {exclude: true}` is documented and omits reasoning text from the response.
  Reasoning still counts toward usage when generated.
- The current no-compression control is
  `plugins: [{id: "context-compression", enabled: false}]`. OpenRouter says compression
  defaults on for models with 8k context or less. The older `transforms: []` control is
  absent from the current chat request schema.
- PDF parsing uses `plugins: [{id: "file-parser", pdf: {engine: "cloudflare-ai"}}]`;
  documented engines are `cloudflare-ai`, `mistral-ocr`, and `native`. Parsed file
  annotations are documented in non-streaming assistant messages and provider-error
  metadata. The current docs do not specify how annotations are delivered in streaming
  responses, so the v1 streaming parse-cache path remains unverified.
- Errors use an `error` object with `code`, `message`, and optional `metadata`. The
  current canonical error types include `context_length_exceeded`, `authentication`,
  `payment_required`, and `rate_limit_exceeded`; common corresponding HTTP statuses are
  401, 402, and 429. Mid-stream errors arrive as SSE chunks after HTTP 200 is committed.

The fake follows the observed models envelope and stream framing (`data:` chunks and
`data: [DONE]`), and returns `usage.cost` for deterministic spend tests.

Sources: [chat completion reference](https://openrouter.ai/docs/api/api-reference/chat/create-a-chat-completion.md),
[streaming guide](https://openrouter.ai/docs/api_reference/streaming.md),
[credits endpoint](https://openrouter.ai/docs/api/api-reference/credits/get-remaining-credits.md),
[key endpoint](https://openrouter.ai/docs/api/api-reference/api-keys/get-current-api-key.md),
[reasoning tokens](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens.md),
[message transforms](https://openrouter.ai/docs/guides/features/message-transforms.md), and
[PDF inputs](https://openrouter.ai/docs/guides/overview/multimodal/pdfs.md).
