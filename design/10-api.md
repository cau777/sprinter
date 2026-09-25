# 10: HTTP API

Status: **Decided**

- Everything lives under `/api` and uses JSON unless stated otherwise.
- Every endpoint except login and `/healthz` requires the session cookie. Without it the
  response is `401`.
- Mutating requests must send `Content-Type: application/json`, or `X-Sprinter: 1` for
  raw uploads. This is the CSRF defense.
- Errors are returned as `{ "error": { "code": "snake_case", "message": "Human readable", "request_id": "…" } }`.
  Every response carries an `X-Request-Id` header (see [13-logging.md](13-logging.md)).
- List endpoints use cursor pagination: `?cursor=&limit=` returns `{ items, next_cursor }`.

## Auth

| Method | Path | Body / Notes |
|---|---|---|
| POST | `/api/auth/login` | `{password}` sets the cookie. Returns `429` with `Retry-After` when rate-limited. |
| POST | `/api/auth/logout` | Ends the current session. |
| GET | `/api/auth/sessions` | Lists sessions, with the current one flagged. |
| DELETE | `/api/auth/sessions/:id` | Revokes one session. |
| DELETE | `/api/auth/sessions` | Revokes all sessions, including the current one. |

## Settings, models and usage

| Method | Path | Notes |
|---|---|---|
| GET | `/api/settings` | All settings. The API key is shown only as `{set: bool, hint: "…a1b2", valid: bool}`. |
| PATCH | `/api/settings` | Partial update. Setting `openrouter_api_key` validates it with OpenRouter before saving. |
| GET | `/api/models` | The OpenRouter model list, trimmed to id, name, context length, pricing, and input modalities. It's cached in memory for 1 h, and `?refresh=1` forces a refresh. |
| GET | `/api/usage` | `{balance, totals:{today,d7,d30,all}, by_model:[…], by_chat:[…], daily:[…]}` |

## Chats

| Method | Path | Notes |
|---|---|---|
| GET | `/api/chats` | Sidebar list: `{id,title,model,updated_at}`, sorted by `updated_at` descending, paginated. |
| POST | `/api/chats` | `{model?}` creates an empty chat. In practice the client usually creates the chat lazily on first send (see below). |
| GET | `/api/chats/:id` | The chat plus **its whole message tree** (flat list with `parent_id` and attachments). The client computes the visible branch from `current_leaf_id`. |
| PATCH | `/api/chats/:id` | `{title?, model?}`. Setting `title` makes `title_source` manual. |
| DELETE | `/api/chats/:id` | Deletes the chat. |
| POST | `/api/chats/:id/switch` | `{message_id}`. The server moves `current_leaf_id` to the most recent leaf under that message and returns the new leaf ID. This powers the `‹ 2/3 ›` picker. |
| GET | `/api/chats/:id/export?format=md\|json` | Returns a file download (`Content-Disposition: attachment`). |

## Messages and generation

| Method | Path | Notes |
|---|---|---|
| POST | `/api/chats/:id/messages` | `{parent_id: string\|null, content, attachment_ids: [], model?, pdf_engine?}`. Inserts the user message plus an assistant message with status `streaming`, sets `current_leaf_id`, starts generation, and returns `{user_message, assistant_message}`. `pdf_engine` is an optional request-level override applied to all uncached PDFs in the outgoing prompt; when omitted, the Settings default is used. **Edit** is the same call with `parent_id` set to the edited message's parent. |
| POST | `/api/chats/new/messages` | Same body, but creates the chat first. Returns the chat as well. This avoids empty chats in the sidebar. |
| POST | `/api/messages/:id/regenerate` | `{model?}` on an assistant message creates a sibling assistant message and starts generation. `model` applies to this retry only and doesn't change the chat's model. |
| POST | `/api/messages/:id/cancel` | Stops the generation and keeps the partial content. |
| GET | `/api/messages/:id/stream` | **SSE**, described below. |

### SSE stream

```
event: snapshot   data: {"content":"…so far…"}                         # always sent first
event: delta      data: {"content":"more"}
event: done       data: {"status":"complete","finish_reason":"stop","usage":{…},"cost":0.0031}
event: error      data: {"status":"error","message":"Provider returned 429"}
: heartbeat                                                            # every 15 s
```

- Subscribing to a message that's already finished returns `snapshot` and then `done`
  right away. The client can therefore always subscribe, whatever the message's state.
- Reasoning text is never streamed (see [01-chat.md](01-chat.md)). While a model is
  reasoning, the stream sends only heartbeats, and the client keeps the streaming glow
  showing with no text yet.
- Several subscribers are allowed, for example the same chat open on a phone and a
  laptop.
- `event: title  data: {"chat_id":"…","title":"…"}` is sent on the stream of a chat's
  first reply once the title is ready. That stream stays open after `done` until the
  title is sent (10 s at most), so a stream **ends** at server close, not at `done` (see
  [01-chat.md](01-chat.md)).
- Common error codes: `no_api_key` (409), `generation_in_progress` (409),
  `too_many_generations` (429), `unsupported_file_type` (415), `file_too_large` (413),
  `not_found` (404), `rate_limited` (429).

## Search

| Method | Path | Notes |
|---|---|---|
| GET | `/api/search?q=` | `[{chat_id, chat_title, message_id\|null, snippet, rank}]`, grouped by chat, top 50. |

## Uploads

| Method | Path | Notes |
|---|---|---|
| PUT | `/api/uploads` | Raw body with the filename in the `X-Filename` header (URL-encoded) and `X-Sprinter: 1`. Streamed to disk. Returns `{id, filename, kind, mime, size}`. Returns `413` if the file is over the limit and `415` for unsupported types. |
| GET | `/api/uploads/:id` | The file content. Security headers are listed in [02-files.md](02-files.md). |
| DELETE | `/api/uploads/:id` | Only allowed while the upload isn't attached to anything (removing a chip in the composer). Otherwise returns `409`. |

## Client logging

| Method | Path | Notes |
|---|---|---|
| POST | `/api/client-log` | `{level, message, stack?, route, app_version}`, at most 8 KB and 20 per minute. Written to the server log (see [13-logging.md](13-logging.md)). |

## Unauthenticated

| Path | Notes |
|---|---|
| `GET /healthz` | Liveness and DB check. |
| `GET /*` | The embedded SPA and assets. Unknown non-API paths serve `index.html`. |
