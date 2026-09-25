# 10: HTTP API

Status: **Decided**

- Everything lives under `/api` and uses JSON unless stated otherwise.
- Every endpoint except login and `/healthz` requires the session cookie. Without it the
  response is `401`.
- Mutating requests must send `Content-Type: application/json`, or `X-Sprinter: 1` for
  raw uploads. This is the CSRF defense.
- Errors are returned as `{ "error": { "code": "snake_case", "message": "Human readable" } }`.
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
| POST | `/api/chats/:id/messages` | `{parent_id: string\|null, content, attachment_ids: [], model?}`. Inserts the user message plus an assistant message with status `streaming`, sets `current_leaf_id`, starts generation, and returns `{user_message, assistant_message}`. **Edit** is the same call with `parent_id` set to the edited message's parent. |
| POST | `/api/chats/new/messages` | Same body, but creates the chat first. Returns the chat as well. This avoids empty chats in the sidebar. |
| POST | `/api/messages/:id/regenerate` | `{model?}` on an assistant message creates a sibling assistant message and starts generation. |
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
- The chat title arrives separately. The client refetches the chat list when a `done`
  event is the chat's first, or the server pushes a `title` event on the first stream.

## Search

| Method | Path | Notes |
|---|---|---|
| GET | `/api/search?q=` | `[{chat_id, chat_title, message_id\|null, snippet, rank}]`, grouped by chat, top 50. |

## Uploads

| Method | Path | Notes |
|---|---|---|
| PUT | `/api/uploads` | Raw body with the filename in the `X-Filename` header (URL-encoded). Streamed to disk. Returns `{id, filename, kind, mime, size}`. Returns `413` if the file is over the limit and `415` for unsupported types. |
| GET | `/api/uploads/:id` | The file content. Security headers are listed in [02-files.md](02-files.md). |
| DELETE | `/api/uploads/:id` | Only allowed while the upload isn't attached to anything (removing a chip in the composer). Otherwise returns `409`. |

## Unauthenticated

| Path | Notes |
|---|---|
| `GET /healthz` | Liveness and DB check. |
| `GET /*` | The embedded SPA and assets. Unknown non-API paths serve `index.html`. |
