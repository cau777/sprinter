# 04: Backend

Status: **Decided**

## Stack

| Concern | Choice |
|---|---|
| Runtime | Tokio |
| HTTP | Axum (tower-http for compression, tracing, limits) |
| Database | SQLite via `sqlx`: WAL mode, FTS5 for search, migrations embedded in the binary |
| HTTP client (OpenRouter) | `reqwest` with rustls (no OpenSSL), streaming SSE parsing |
| Static assets | `rust-embed`, with the Vite build embedded in the binary |
| Allocator | `mimalloc`. musl's default allocator is slow under concurrency. |
| Queries | Runtime `sqlx::query_as` with `FromRow`. **No** compile-time `query!` macros, so there's no `DATABASE_URL` or `.sqlx` offline cache to maintain. |
| SQLite build | `libsqlite3-sys` bundled (FTS5 enabled; check this at scaffold) |
| Crypto | `argon2`, `chacha20poly1305`, `sha2`, `rand` |
| TS types | `ts-rs` derives on API structs, exported by `cargo run --bin gen-types` |
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

## Configuration

The canonical env var list and `DATA_DIR` layout are in
[08-deployment.md](08-deployment.md). The OpenRouter API key and model defaults are
**not** env vars. They live in the DB and are edited on the Settings page.

## HTTP hardening

- **CSP** on the SPA:
  `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; worker-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'`.
  KaTeX and Mermaid need inline styles. No third-party origins are loaded.
- Also sent: `Referrer-Policy: no-referrer`, `X-Content-Type-Options: nosniff`, and
  `Permissions-Policy` denying the camera, microphone and geolocation.
- JSON request bodies are capped at 1 MB. Upload bodies are governed by
  [02-files.md](02-files.md).
- **Logs:** see [13-logging.md](13-logging.md). Secrets are never logged. Message
  content appears only as truncated previews.

## Background tasks

One Tokio interval scheduler runs:
- **Hourly:** upload GC and the `tmp/` sweep ([02-files.md](02-files.md)), and deleting
  expired sessions.
- **Backups:** managed by the host outside Sprinter. Back up the mounted `DATA_DIR`
  consistently, including the SQLite database and uploads ([08-deployment.md](08-deployment.md)).
- **Hourly:** refreshing the model list cache.
