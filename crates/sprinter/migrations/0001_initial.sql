-- Sprinter v1 schema. The migration runs on startup and creates the complete schema
-- before the server starts accepting requests.

CREATE TABLE settings (
  key         TEXT PRIMARY KEY,
  value       TEXT NOT NULL,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE sessions (
  id            TEXT PRIMARY KEY,
  token_hash    BLOB NOT NULL UNIQUE,
  user_agent    TEXT,
  ip            TEXT,
  created_at    INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  expires_at    INTEGER NOT NULL
);
CREATE INDEX sessions_expires ON sessions(expires_at);

CREATE TABLE chats (
  id               TEXT PRIMARY KEY,
  title            TEXT,
  title_source     TEXT NOT NULL DEFAULT 'auto'
                   CHECK (title_source IN ('auto', 'manual')),
  model            TEXT NOT NULL,
  current_leaf_id  TEXT REFERENCES messages(id) ON DELETE SET NULL,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);
CREATE INDEX chats_updated ON chats(updated_at DESC);

CREATE TABLE messages (
  id                 TEXT PRIMARY KEY,
  chat_id            TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
  parent_id          TEXT REFERENCES messages(id) ON DELETE CASCADE,
  role               TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
  content            TEXT NOT NULL DEFAULT '',
  status             TEXT NOT NULL DEFAULT 'complete'
                     CHECK (status IN ('streaming', 'complete', 'cancelled', 'error', 'interrupted')),
  error              TEXT,
  model              TEXT,
  generation_id      TEXT,
  finish_reason      TEXT,
  prompt_tokens      INTEGER,
  completion_tokens  INTEGER,
  reasoning_tokens   INTEGER,
  cost               REAL,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL
);
CREATE INDEX messages_chat ON messages(chat_id, created_at);
CREATE INDEX messages_parent ON messages(parent_id, created_at);

CREATE TABLE uploads (
  id          TEXT PRIMARY KEY,
  sha256      TEXT NOT NULL,
  filename    TEXT NOT NULL,
  mime        TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('image', 'pdf', 'text')),
  size        INTEGER NOT NULL CHECK (size >= 0),
  created_at  INTEGER NOT NULL
);
CREATE INDEX uploads_sha ON uploads(sha256);

CREATE TABLE message_attachments (
  message_id   TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
  upload_id    TEXT NOT NULL REFERENCES uploads(id),
  position     INTEGER NOT NULL CHECK (position >= 0),
  pdf_engine   TEXT CHECK (pdf_engine IS NULL OR pdf_engine IN ('cloudflare-ai', 'mistral-ocr', 'native')),
  parse_cache  TEXT,
  PRIMARY KEY (message_id, upload_id)
);
CREATE INDEX message_attachments_upload ON message_attachments(upload_id);

CREATE TABLE usage_events (
  id                TEXT PRIMARY KEY,
  chat_id           TEXT REFERENCES chats(id) ON DELETE SET NULL,
  source            TEXT NOT NULL CHECK (source IN ('title')),
  model             TEXT NOT NULL,
  prompt_tokens     INTEGER,
  completion_tokens INTEGER,
  cost              REAL,
  created_at        INTEGER NOT NULL
);
CREATE INDEX usage_events_chat ON usage_events(chat_id, created_at);
CREATE INDEX usage_events_created ON usage_events(created_at);

CREATE TABLE usage_rollup (
  day                TEXT NOT NULL,
  model              TEXT NOT NULL,
  prompt_tokens      INTEGER NOT NULL DEFAULT 0,
  completion_tokens  INTEGER NOT NULL DEFAULT 0,
  cost               REAL NOT NULL DEFAULT 0,
  PRIMARY KEY (day, model)
);

CREATE VIRTUAL TABLE search_fts USING fts5(
  text,
  chat_id UNINDEXED,
  message_id UNINDEXED,
  tokenize = 'unicode61 remove_diacritics 2'
);

-- A title is represented by a null message_id. A title row is replaced whenever
-- the title changes and removed when its chat is deleted.
CREATE TRIGGER chats_search_title_insert AFTER INSERT ON chats
WHEN NEW.title IS NOT NULL
BEGIN
  INSERT INTO search_fts(text, chat_id, message_id)
  VALUES (NEW.title, NEW.id, NULL);
END;

CREATE TRIGGER chats_search_title_update AFTER UPDATE OF title ON chats
BEGIN
  DELETE FROM search_fts
  WHERE chat_id = OLD.id AND message_id IS NULL;
  INSERT INTO search_fts(text, chat_id, message_id)
  SELECT NEW.title, NEW.id, NULL WHERE NEW.title IS NOT NULL;
END;

CREATE TRIGGER chats_search_title_delete AFTER DELETE ON chats
BEGIN
  DELETE FROM search_fts
  WHERE chat_id = OLD.id AND message_id IS NULL;
END;

-- Streaming content is not indexed on every flush. Transitioning to a terminal
-- status inserts the final content. Other updates replace their existing row.
CREATE TRIGGER messages_search_insert AFTER INSERT ON messages
WHEN NEW.status <> 'streaming'
BEGIN
  INSERT INTO search_fts(text, chat_id, message_id)
  VALUES (NEW.content, NEW.chat_id, NEW.id);
END;

CREATE TRIGGER messages_search_update AFTER UPDATE OF content, status ON messages
BEGIN
  DELETE FROM search_fts WHERE message_id = OLD.id;
  INSERT INTO search_fts(text, chat_id, message_id)
  SELECT NEW.content, NEW.chat_id, NEW.id WHERE NEW.status <> 'streaming';
END;

CREATE TRIGGER messages_search_delete AFTER DELETE ON messages
BEGIN
  DELETE FROM search_fts WHERE message_id = OLD.id;
END;
