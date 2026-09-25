# 04: Backend

Status: **Decided** (stack). The schema will be written after the file handling is
finalized.

## Stack

| Concern | Choice |
|---|---|
| Runtime | Tokio |
| HTTP | Axum (tower-http for compression, tracing, limits) |
| Database | SQLite via `sqlx`: WAL mode, FTS5 for search, migrations embedded in the binary |
| HTTP client (OpenRouter) | `reqwest` with rustls (no OpenSSL), streaming SSE parsing |
| Static assets | `rust-embed`, with the Vite build embedded in the binary |
| Allocator | `mimalloc`. musl's default allocator is slow under concurrency. |
| Image | Static `x86_64/aarch64-unknown-linux-musl` binary on `scratch` (or distroless/static), running as a non-root user |

## Memory

Axum adds very little on top of Tokio and hyper. A small Axum + sqlx service typically
idles in the **~5–15 MB RSS** range, which is comparable to or below actix-web. Actual
memory use comes from our own choices, so we set these deliberately:

- **Tokio workers:** capped (for example 2–4) via config. A single user doesn't need one
  worker per core.
- **SQLite:** a small connection pool (1 writer plus a few readers), with a bounded
  `cache_size` (for example 8 MB) and `mmap_size` left modest.
- **Uploads:** streamed to disk and never buffered whole in memory (see
  [02-files.md](02-files.md)).
- **Generation buffers:** each in-flight generation holds its text so far (KBs), and the
  buffer is dropped once complete.
- **OpenRouter requests:** attachments are base64-encoded into the request body, which is
  the largest transient allocation. File size limits in 02-files.md bound it.

Target: **under 30 MB RSS idle, under 100 MB under normal use.** Peaks around 200 MB are possible when a request carries the maximum attachment load (100 MB raw, base64-encoded).

## Configuration (env vars)

| Var | Default | Purpose |
|---|---|---|
| `SPRINTER_PASSWORD` | _(required)_ | Master password. The server refuses to start without it. |
| `DATA_DIR` | `/var/lib/sprinter` | All persistent state |
| `PORT` | `8080` | Listen port |
| `BIND` | `0.0.0.0` | Listen address |

The OpenRouter API key and model defaults are **not** env vars. They live in the DB and
are edited on the Settings page.

## DATA_DIR layout (initial)

```
$DATA_DIR/
  sprinter.db        # SQLite (plus -wal / -shm)
  uploads/           # see 02-files.md
  tmp/               # in-progress uploads, on the same filesystem so rename is atomic
```
