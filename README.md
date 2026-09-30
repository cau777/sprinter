# Sprinter

Sprinter is a self-hosted AI chat application. It serves the web app and API from one
container and stores its SQLite database, uploads, and logs under `DATA_DIR`.  
OpenRouter is used as the model provider. We only use ZDR models and tool calls, so data is guaranteed to be only in your database, it won't end up in the knowledgebase of the next model iteration.

## Motivation

Recent drama surfaced the fact that, if models train on your data, they may remember it later and wipe out your copyright. 

The obvious answer is local models, but they are often less capable and require expensive hardware.

Another option is ZDR endpoints, like calling GPT through Azure instead of OpenAI. These are usually slightly more expensive, but could be worth it for your privacy.

However, it is difficult to know when your ZDR ends. For example, calling Claude through the ZDR endpoint on OpenRouter may send your data to a third-party search provider if the model uses a web_search tool.

The app tries to make ZDR as effortless as possible, while not limiting your model selection. If a feature is not ZDR, it is not shown.

## Development

Install the frontend dependencies and the Rust file watcher once:

```sh
npm install
cargo install cargo-watch
```

Then start both development servers:

```sh
npm run dev
```

Open `http://localhost:5173`. Vite reloads frontend changes and `cargo watch` restarts
the API when Rust sources change. Vite proxies `/api` and `/healthz` to the API at
`http://127.0.0.1:8080`, so the browser continues to use the normal same-origin API
paths and session cookies. Development state is stored in `./data`, and the command
uses a local development password with insecure cookies. Override the API proxy target
with `VITE_API_PROXY_TARGET`, or run `npm run dev:api` and `npm run dev:web` separately.

## Configuration

Set these environment variables on the container:


| Variable                    | Default                        | Purpose                                                                    |
| --------------------------- | ------------------------------ | -------------------------------------------------------------------------- |
| `SPRINTER_PASSWORD`         | Required                       | Master password used for login and key encryption.                         |
| `DATA_DIR`                  | `/var/lib/sprinter`            | Persistent database, uploads, and logs.                                    |
| `BIND`                      | `0.0.0.0`                      | Address the server listens on.                                             |
| `PORT`                      | `8080`                         | HTTP port inside the container.                                            |
| `TRUSTED_PROXIES`           | Empty                          | Comma-separated proxy IPs or CIDRs allowed to supply forwarded client IPs. |
| `WORKER_THREADS`            | `2`                            | Tokio worker thread count.                                                 |
| `RUST_LOG`                  | `info`                         | Log filter, for example `info,sprinter::generation=debug`.                 |
| `LOG_KEEP_DAYS`             | `30`                           | Number of daily log files to retain.                                       |
| `SPRINTER_INSECURE_COOKIES` | `false`                        | Development setting to allow cookies over plain HTTP.                      |
| `OPENROUTER_BASE_URL`       | `https://openrouter.ai/api/v1` | OpenRouter API base URL; override for local testing.                       |


The OpenRouter key is managed in Settings. A balance is shown only when OpenRouter
returns a per-key `limit_remaining` value; keys without a spend limit show balance as
unavailable.

## Backups and restore

Backups are managed by the host or deployment platform. Back up the mounted `DATA_DIR`
with a method that captures the SQLite database consistently and includes `uploads/`.
Sprinter does not create or retain database snapshots. Use the backup system's restore
procedure with Sprinter stopped, restoring the database and its matching uploads.

## Operations

The container health check runs `/sprinter healthcheck`, which opens the configured
database and verifies it is reachable. Logs are written to stdout and to daily files in
`DATA_DIR/logs/`. Those files can contain short previews of chat messages and should be
protected like the database; see [design/13-logging.md](design/13-logging.md).

Releases tagged `vX.Y.Z` are built for `linux/amd64` and `linux/arm64` and published to
`ghcr.io/cau777/sprinter` with version and `latest` tags.