# 08: Deployment, operations and backups

Status: **Decided**

## Topology

```
browser (PWA) ──HTTPS──▶ reverse proxy (Caddy / Traefik / nginx / CF Tunnel) ──HTTP──▶ sprinter:8080
                                                                                         │
                                                                               DATA_DIR (volume)
```

- Sprinter serves **plain HTTP**. TLS is handled by the user's reverse proxy.
- The browser only sees HTTPS, so the `Secure` cookie flag and the service worker work
  normally.

## Container image

- A multi-stage build: Node builds `web/`, then Rust builds a static musl binary with the
  web build embedded. The final stage is `FROM scratch` with CA certificates, the binary,
  and a non-root UID.
- Architectures: **linux/amd64** and **linux/arm64**.
- Built by GitHub Actions and published to GHCR (`ghcr.io/cau777/sprinter`), tagged with
  the semver and `latest`.
- Target image size: under 20 MB.
- The binary has a `healthcheck` subcommand, because `scratch` has no curl.
  `GET /healthz` is unauthenticated and returns 200 when the DB is reachable.

## Reverse proxy requirements

These go in the README with example Caddy and nginx snippets.

| Concern | Requirement |
|---|---|
| Client IP for login rate limiting | Sprinter trusts `X-Forwarded-For` **only** from the addresses in `TRUSTED_PROXIES` (CIDR list). Otherwise it uses the socket address. This stops an attacker from spoofing the header to get around rate limits. |
| SSE streaming | The proxy must not buffer responses. Sprinter sends `X-Accel-Buffering: no` and `Cache-Control: no-cache`, plus an SSE heartbeat comment every 15 s so idle-timeout proxies (Cloudflare ~100 s) don't cut the stream. |
| Upload size | The proxy's body limit must be at least the largest upload (default 50 MB PDF). Cloudflare's free plan caps request bodies at 100 MB. |
| Timeouts | Normal request timeouts are fine because generation is decoupled from requests (see [01-chat.md](01-chat.md)). |

## Env vars (full list)

| Var | Default | Purpose |
|---|---|---|
| `SPRINTER_PASSWORD` | _(required)_ | Master password |
| `DATA_DIR` | `/var/lib/sprinter` | Persistent state |
| `BIND` / `PORT` | `0.0.0.0` / `8080` | Listen address |
| `TRUSTED_PROXIES` | _(empty)_ | CIDRs whose `X-Forwarded-For` is trusted |
| `BACKUP_KEEP` | `7` | Number of daily DB snapshots to keep |
| `WORKER_THREADS` | `2` | Tokio worker threads |
| `RUST_LOG` | `info` | Log level filter for the log files and stdout. See [13-logging.md](13-logging.md). |
| `LOG_KEEP_DAYS` | `30` | Number of daily log files kept in `DATA_DIR/logs` |
| `SPRINTER_INSECURE_COOKIES` | `false` | Development only: drops the `Secure` cookie flag for plain-HTTP localhost |
| `OPENROUTER_BASE_URL` | `https://openrouter.ai/api/v1` | Override for tests (fake OpenRouter). See [11-testing.md](11-testing.md). |

## Backups

The **host** backs up the mounted `DATA_DIR` (volume snapshots, restic, rsync, and so on).
Sprinter makes sure a plain file copy of that directory is always consistent:

- A copy of a live SQLite DB in WAL mode can be torn. So once a day (and on graceful
  shutdown), Sprinter writes a consistent snapshot with `VACUUM INTO` to
  `DATA_DIR/backups/sprinter-YYYY-MM-DD.db` (written to a temp file, then renamed). It
  keeps `BACKUP_KEEP` snapshots.
- Uploads are content-addressed and immutable, and each is written through a temp file
  and an atomic rename. That makes copying `uploads/` inherently safe.
- **Recommended host backup set:** `backups/` and `uploads/`. The live `sprinter.db*`
  files and `logs/` can be excluded.
- **Restore:** stop the container, copy the chosen snapshot to `sprinter.db`, delete any
  `sprinter.db-wal` and `-shm`, then start the container.

`DATA_DIR` layout:

```
$DATA_DIR/
  sprinter.db, sprinter.db-wal, sprinter.db-shm   # live DB
  backups/sprinter-YYYY-MM-DD.db                  # consistent daily snapshots
  uploads/ab/ab12…                                # content-addressed uploads
  logs/sprinter.log.YYYY-MM-DD                    # daily text logs (13-logging)
  tmp/                                            # in-progress uploads and snapshots
```

## Export (per chat)

The chat menu has an **Export** option with two formats:
- **Markdown:** the currently visible branch, with attachments listed by filename, model
  names, and timestamps. Meant for reading and sharing.
- **JSON:** the full message tree (all branches), including models, usage and cost, and
  attachment metadata. Meant for archiving or re-importing later.

Attachment files are not embedded in either format. Import is not in v1.
The file is named `<slugified-title>-<YYYY-MM-DD>.md|json`.
