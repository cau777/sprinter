# 01: Chat model and generation

Status: **Decided** (behavior). The storage schema will be settled in the backend doc.

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

1. `POST /chats/:id/messages` (or a regenerate/edit endpoint) validates the request and
   inserts the user message plus an empty assistant message with `status = streaming`. It
   spawns a background task and returns the new message IDs right away.
2. The background task calls OpenRouter with `stream: true` and appends deltas to an
   in-memory buffer, which fans out to subscribers. It flushes the accumulated content to
   the DB periodically (for example every ~1 s) and on completion.
3. The client subscribes to `GET /messages/:id/stream`, which is SSE. A subscriber first
   gets everything generated so far, then live deltas. Reconnecting after a drop gives the
   same result, so reattaching is seamless.
4. On completion the message gets `status = complete` and records usage and cost data from
   OpenRouter, finish reason, and model. On failure it gets `status = error` with the
   message.
5. The user can **stop** a generation explicitly with `POST /messages/:id/cancel`. This is
   the only thing that cancels it. The partial content is kept and the status becomes
   `cancelled`.
6. On server start, any message still in `streaming` status (left over from a crash or
   restart) is marked `interrupted`.

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

After the first assistant reply completes, a background task asks the title model for a
short title. The user can rename the chat at any time. A manual rename is never
overwritten.

## Search

Search uses SQLite FTS5 over message content and chat titles (assuming SQLite, which the
backend doc will confirm). Results link to the chat, and to the matching message where
possible.
