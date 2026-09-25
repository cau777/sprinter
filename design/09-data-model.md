# 09: Data model (SQLite)

Status: **Decided**

Conventions:
- IDs are **UUIDv7** stored as `TEXT`. They're time-ordered, so they sort by creation
  and index well.
- Timestamps are `INTEGER` Unix milliseconds (UTC).
- `PRAGMA foreign_keys = ON`, `journal_mode = WAL`, `synchronous = NORMAL`.
- Migrations are managed by `sqlx migrate` and embedded in the binary. They run
  automatically at startup.

## Tables

```sql
CREATE TABLE settings (
  key         TEXT PRIMARY KEY,           -- e.g. 'default_model', 'favorite_models'
  value       TEXT NOT NULL,              -- JSON
  updated_at  INTEGER NOT NULL
);
-- openrouter_api_key is stored as JSON {salt, nonce, ciphertext}; see 07-settings.md

CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,
  token_hash    BLOB NOT NULL UNIQUE,     -- SHA-256 of the cookie token
  user_agent    TEXT,
  ip            TEXT,
  created_at    INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  expires_at    INTEGER NOT NULL
);

CREATE TABLE chats (
  id               TEXT PRIMARY KEY,
  title            TEXT,                  -- NULL until the auto title arrives
  title_source     TEXT NOT NULL DEFAULT 'auto' CHECK (title_source IN ('auto','manual')),
  model            TEXT NOT NULL,         -- model used for the next generation in this chat
  current_leaf_id  TEXT REFERENCES messages(id) ON DELETE SET NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL       -- bumped on every new message; sidebar sort key
);
CREATE INDEX chats_updated ON chats(updated_at DESC);

CREATE TABLE messages (
  id                 TEXT PRIMARY KEY,
  chat_id            TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
  parent_id          TEXT REFERENCES messages(id) ON DELETE CASCADE,
  role               TEXT NOT NULL CHECK (role IN ('user','assistant')),
  content            TEXT NOT NULL DEFAULT '',
  status             TEXT NOT NULL DEFAULT 'complete'
                     CHECK (status IN ('streaming','complete','cancelled','error','interrupted')),
  error              TEXT,
  model              TEXT,                -- assistant: model that produced it
  generation_id      TEXT,                -- OpenRouter generation id
  finish_reason      TEXT,
  prompt_tokens      INTEGER,
  completion_tokens  INTEGER,
  reasoning_tokens   INTEGER,             -- billed even though reasoning text is hidden
  cost               REAL,                -- USD, as reported by OpenRouter
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL
);
CREATE INDEX messages_chat   ON messages(chat_id, created_at);
CREATE INDEX messages_parent ON messages(parent_id, created_at);

CREATE TABLE uploads (
  id          TEXT PRIMARY KEY,
  sha256      TEXT NOT NULL,              -- file lives at uploads/<sha[0..2]>/<sha>
  filename    TEXT NOT NULL,
  mime        TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('image','pdf','text')),
  size        INTEGER NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX uploads_sha ON uploads(sha256);

CREATE TABLE message_attachments (
  message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  upload_id    TEXT NOT NULL REFERENCES uploads(id),
  position     INTEGER NOT NULL,
  parse_cache  TEXT,                      -- OpenRouter PDF annotation JSON
  PRIMARY KEY (message_id, upload_id)
);
CREATE INDEX message_attachments_upload ON message_attachments(upload_id);

-- Auxiliary spend (title generation), so usage totals are complete
CREATE TABLE usage_events (
  id          TEXT PRIMARY KEY,
  chat_id     TEXT REFERENCES chats(id) ON DELETE SET NULL,
  source      TEXT NOT NULL CHECK (source IN ('title')),
  model       TEXT NOT NULL,
  prompt_tokens INTEGER, completion_tokens INTEGER, cost REAL,
  created_at  INTEGER NOT NULL
);

-- Spend from deleted chats, folded in on delete so lifetime totals stay accurate
CREATE TABLE usage_rollup (
  day                TEXT NOT NULL,       -- 'YYYY-MM-DD' (UTC)
  model              TEXT NOT NULL,
  prompt_tokens      INTEGER NOT NULL DEFAULT 0,
  completion_tokens  INTEGER NOT NULL DEFAULT 0,
  cost               REAL NOT NULL DEFAULT 0,
  PRIMARY KEY (day, model)
);
```

## Full-text search

```sql
CREATE VIRTUAL TABLE search_fts USING fts5(
  text, chat_id UNINDEXED, message_id UNINDEXED,
  tokenize = 'unicode61 remove_diacritics 2'
);
```

- Triggers keep it in sync with `messages.content` (insert, update, delete) and
  `chats.title`. A title row has `message_id = NULL`.
- Streaming messages are indexed when they reach a final status, not on every flush.
- Queries use FTS5 `bm25()` ranking and `snippet()` for result previews, grouped by chat.

## Invariants and notes

- **Tree:** a message's `parent_id` must belong to the same chat (enforced in the app).
  A chat can have several roots, because editing the first message creates a sibling
  root.
- **Deleting a chat** cascades to its messages and attachments. Orphaned `uploads` rows
  and files are removed by the GC sweep in [02-files.md](02-files.md).
- **Deleting a single message or branch** is not a v1 feature. The tree is append-only.
- **Spend survives deleting a chat.** In the same transaction as the delete, the chat's
  message costs are summed per `(day, model)` and upserted into `usage_rollup`. Its
  `usage_events` are kept, with `chat_id` set to NULL. Usage totals are computed as
  `messages + usage_events + usage_rollup`. A deleted chat's spend disappears only from
  the by-chat breakdown.
- The OpenRouter model list is **not** stored. It's cached in memory and refreshed
  hourly (see [10-api.md](10-api.md)).
