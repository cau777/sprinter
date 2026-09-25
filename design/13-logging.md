# 13: Logging

Status: **Decided**

## Goals

- **Comprehensive:** every request, generation, upload, OpenRouter call, background task,
  state change, and error leaves a trace. It should be possible to reconstruct what
  happened to any chat or message from the logs alone.
- **Readable:** plain text lines you can `tail`, `less`, and `grep` in `DATA_DIR/logs`.
- **Correlated:** request IDs and entity IDs (chat, message, upload, generation) appear
  on every related line.
- **Secrets never logged:** the password, API key, session tokens, and cookies never
  appear, at any level.

## Outputs

| Output | Content | Notes |
|---|---|---|
| `DATA_DIR/logs/sprinter.log.YYYY-MM-DD` | All lines at or above the configured level | Rotates daily at 00:00 UTC. Files older than `LOG_KEEP_DAYS` are deleted. |
| stdout | The same lines | So `docker logs` works. ANSI colors only when stdout is a TTY. |

- Built with `tracing` and `tracing-subscriber`: one `fmt` layer for stdout, and one for
  the file through `tracing-appender`'s daily `RollingFileAppender` with
  `max_log_files = LOG_KEEP_DAYS`.
- The file writer is **non-blocking** (a background thread) with `lossy = false`. Under
  back-pressure it slows the caller rather than dropping lines, which for a single user
  never matters in practice.
- If `DATA_DIR/logs` can't be created or written, Sprinter logs one error to stdout and
  keeps running with stdout only. Logging never stops the app from starting.
- Logs are **not** part of the recommended backup set ([08-deployment.md](08-deployment.md)).

### Configuration

| Var | Default | Purpose |
|---|---|---|
| `RUST_LOG` | `info` | Level filter for both outputs (standard `EnvFilter` syntax, for example `info,sprinter::generation=debug`) |
| `LOG_KEEP_DAYS` | `30` | Number of daily log files to keep |

## Line format

```
<UTC timestamp, ms>  <LEVEL> <span context>: <target>: <message> <key=value fields>
```

Examples:

```
2026-09-25T14:03:11.482Z  INFO sprinter::server: starting version=0.1.0 bind=0.0.0.0:8080 data_dir=/var/lib/sprinter workers=2 log_keep_days=30 trusted_proxies=[172.18.0.0/16]
2026-09-25T14:03:40.120Z  WARN request{rid=01J8Z4K2 ip=203.0.113.7}: sprinter::auth: login failed attempts_in_window=2
2026-09-25T14:03:44.901Z  INFO request{rid=01J8Z4K9 ip=203.0.113.7}: sprinter::auth: login ok session=ses_7f3a ua="Mozilla/5.0 (Linux; Android 15; Pixel 8)"
2026-09-25T14:05:02.310Z  INFO request{rid=01J8Z4R1 method=POST route=/api/chats/{id}/messages chat=0192c3e1}: sprinter::api::messages: message sent message=0192c3e2 parent=- attachments=1[pdf] chars=58 preview="Summarize the termination clause in this lease and…"
2026-09-25T14:05:02.312Z  INFO generation{gen=0192c3e3 chat=0192c3e1 model=anthropic/claude-sonnet-5}: sprinter::generation: started path_len=1 attachments_bytes=3.1MB pdf_engine=cloudflare-ai
2026-09-25T14:05:03.954Z  INFO generation{gen=0192c3e3 …}: sprinter::openrouter: first token ttft_ms=1642 provider=Anthropic or_id=gen-1727273103-abc
2026-09-25T14:05:14.771Z  INFO generation{gen=0192c3e3 …}: sprinter::generation: completed status=complete finish=stop duration_ms=12459 prompt_tokens=4120 completion_tokens=611 reasoning_tokens=0 cost=0.021570 subscribers=1 preview="The termination clause (§14.2) allows either party to…"
2026-09-25T14:05:14.772Z  INFO request{rid=01J8Z4R1 …}: sprinter::http: completed status=200 latency_ms=3
2026-09-25T14:05:15.020Z  INFO title{chat=0192c3e1 model=google/gemini-flash-2}: sprinter::titles: title set title="Lease termination clause" cost=0.000041 duration_ms=708
```

- Levels are padded to a fixed width. Long IDs are shown in full in the file. The
  examples shorten them for readability.
- Field values containing spaces or quotes are quoted and escaped. **Newlines inside
  values are escaped as `\n`**, so every event is exactly one line and grep works.
- A `request` span is opened for every HTTP request, and a `generation` or `title` span
  for every background job. Every line emitted inside a span carries its fields.

## Request IDs

- Every request gets a request ID (ULID, shortened in display) that is:
  - on every log line for that request
  - returned in the `X-Request-Id` response header
  - included in API error bodies as `error.request_id`, and shown in UI error toasts
    ("Something went wrong · ref 01J8Z4R1")

  So a problem seen on your phone can be found with `grep 01J8Z4R1`.
- A generation's span records the `rid` of the request that started it (`started_by`).
  Generation lines can then be traced back to the send or regenerate call.

## Content in logs

Per your choice, the logs contain **metadata plus truncated content**:

| Content | Logged as |
|---|---|
| User message text | `preview=`, the first 200 characters, newlines escaped, followed by `…` if cut, plus `chars=` with the full length |
| Assistant reply text | `preview=` on the `completed`, `cancelled`, `error`, and `interrupted` lines, same rules. Streaming deltas are never logged. |
| Chat titles | In full (they're short) |
| Search queries | The first 200 characters |
| Custom instructions | On change: `chars=` and the first 200 characters |
| Filenames | In full |
| File contents, base64 data, PDF annotations | **Never**. Only size, kind, sha256 prefix, and dimensions where known. |
| OpenRouter request and response bodies | **Never** in full. The request is summarized (model, message count, parts by kind, bytes, plugins, headers without `Authorization`). The response is summarized (status, provider, IDs, usage, finish reason). |

This makes the log files **sensitive**, since they contain fragments of your
conversations. They live under `DATA_DIR` with `0600` file and `0700` directory
permissions, just like the database.

## Never logged, at any level

- `SPRINTER_PASSWORD`, submitted passwords, and password hashes
- The OpenRouter API key (in any form, including the hint; only `key_set=true` or
  `false` is logged)
- Session tokens, the `Cookie` and `Set-Cookie` headers, and the `Authorization` header
- The derived encryption key

How this is enforced:
- Secrets are wrapped in `secrecy::SecretString` (and a similar wrapper for the token),
  whose `Debug` and `Display` print `[REDACTED]`.
- Headers are never logged wholesale. Request logging uses an allow-list: method, route
  template, status, latency, bytes, IP, user agent.
- A backend test (see [11-testing.md](11-testing.md)) captures all log output during
  login, key save, and a generation, and asserts that none of the secrets appear.

## Levels

| Level | Used for |
|---|---|
| **ERROR** | Something broke that shouldn't: DB or IO errors, a panicked task, a failed snapshot, an unreadable config, an internal error behind a 500. Always includes the error chain. |
| **WARN** | Expected-but-notable failures: a provider error (401, 402, 429, 5xx, context too long), a failed login, a rate limit hit, a rejected upload (413, 415), a generation marked `interrupted`, a password-change rotation, an unreadable API key, a request slower than 2 s, and client-reported errors. |
| **INFO** | Normal lifecycle: startup and shutdown, each completed API request, login and logout, chat create, rename and delete, message sent, generation start, first token, and completion, titles, uploads stored or deduped, settings changed (keys and non-secret values), branch switch, export, GC, snapshot, and model cache refresh results. |
| **DEBUG** | High volume or low value: static assets and `/healthz`, SSE subscribe and unsubscribe, DB flushes during streaming, OpenRouter request and response summaries, cache hits and misses, and session expiry refreshes. |
| **TRACE** | Per-chunk streaming counters and SQL statements (through sqlx's own logging). Only for deep debugging. |

## Event catalog (INFO and above)

Fields listed are in addition to span context.

| Area | Event | Key fields |
|---|---|---|
| Server | `starting` | version, bind, data_dir, workers, log level, keep days, trusted proxies, `key_set` |
| | `migrations applied` | from and to version, duration_ms |
| | `password rotated` (WARN) | sessions_revoked, `api_key_unreadable=true` |
| | `interrupted on startup` (WARN) | count, message IDs |
| | `shutting down` / `stopped` | in-flight generations, how many finished or were interrupted, duration_ms |
| HTTP | `completed` | method, route, status, latency_ms, req and resp bytes |
| | `slow request` (WARN) | the same, when latency is over 2 s (SSE and uploads excluded) |
| Auth | `login ok`, `login failed` (WARN), `rate limited` (WARN), `logout`, `session revoked` | session ID (not the token), ip, ua, attempts, retry_after |
| Settings | `settings changed` | keys changed, non-secret new values (models, pdf_engine, limits), instructions preview |
| | `api key saved` / `api key rejected` (WARN) | `valid`, the OpenRouter status on rejection |
| Chats | `chat created`, `chat renamed`, `chat deleted` | chat, title, messages removed, uploads orphaned, spend rolled up |
| | `branch switched` | chat, from_leaf, to_leaf |
| | `exported` | chat, format, bytes |
| Messages | `message sent` | message, parent, edit (bool), attachments (count and kinds), chars, preview |
| | `regenerate` | message, sibling_of, model override |
| Generation | `started` | gen, chat, model, path_len, attachment bytes, pdf_engine, `started_by` |
| | `first token` | ttft_ms, provider, OpenRouter generation ID |
| | `completed` / `cancelled` / `error` (WARN) / `interrupted` (WARN) | status, finish, duration_ms, tokens (prompt, completion, reasoning), cost, subscribers, preview, error message and provider status |
| | `images omitted` (WARN) | count, model (no vision) |
| Titles | `title set` / `title fallback` (WARN) | title, model, cost, duration_ms, error |
| Uploads | `upload stored` / `upload deduped` | upload, filename, kind, mime, size, sha256 prefix, duration_ms |
| | `upload rejected` (WARN) | reason, declared filename, bytes received |
| | `upload deleted` | upload, reason (user, gc) |
| Search | `search` | query preview, results, duration_ms |
| Usage | `usage viewed` (DEBUG) and `balance fetched` | balance, cache hit |
| Tasks | `gc` | orphans removed, bytes freed, tmp files swept, sessions expired |
| | `snapshot written` / `snapshot failed` (ERROR) | path, bytes, duration_ms, pruned |
| | `models refreshed` / `models refresh failed` (WARN) | count, duration_ms |
| Client | `client error` (WARN) | message, stack preview (first 500 characters), route, ua, app version |

## Client-reported errors

The PWA runs on phones where devtools aren't available, so client errors are sent to
the server log:

- `POST /api/client-log` with `{level, message, stack?, route, app_version}`. It needs
  authentication and is rate-limited to 20 per minute, and the body is capped at 8 KB.
- The client reports unhandled exceptions, unhandled promise rejections, React error
  boundary catches, and failed API calls with status ≥ 500 (including their
  `request_id`).
- These are logged under the target `sprinter::client` at WARN.

## Out of scope for v1

- A log viewer in the app (use `docker exec` or the mounted volume).
- Metrics or OpenTelemetry export.
- Structured JSON logs. The format can be switched later with a `LOG_FORMAT=json` env var
  if logs are ever shipped somewhere.
