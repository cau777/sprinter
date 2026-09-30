# Sprinter

Sprinter is a self-hosted AI chat application. It serves the web app and API from one
container and stores its SQLite database, uploads, and logs under `DATA_DIR`.
OpenRouter is used for model discovery and chat generation; the API key is encrypted
before it is stored.

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

## Run with Docker

Use a strong master password and keep the data volume persistent:

```sh
docker volume create sprinter-data
docker run -d \
  --name sprinter \
  --restart unless-stopped \
  -p 127.0.0.1:8080:8080 \
  -v sprinter-data:/var/lib/sprinter \
  -e SPRINTER_PASSWORD='replace-with-a-long-random-password' \
  ghcr.io/cau777/sprinter:latest
```

Put Sprinter behind an HTTPS reverse proxy before using it from a browser. The session
cookie is `Secure` by default. The proxy can run on the host or on a private container
network; only expose port 8080 to that proxy. Then open your HTTPS hostname, sign in with
the master password, and save an OpenRouter API key in Settings.

For local HTTP development only, set `SPRINTER_INSECURE_COOKIES=true`. Do not use that
setting on a public deployment.

## Reverse proxy

### Caddy

If Caddy and Sprinter share a private Docker network, Caddy can provide automatic HTTPS:

```caddyfile
chat.example.com {
    reverse_proxy sprinter:8080
}
```

Set `TRUSTED_PROXIES` on Sprinter to the proxy container's network CIDR, such as
`172.20.0.0/16`. Find the actual network with `docker network inspect`. Caddy forwards
the client address, and Sprinter uses forwarded addresses only when the direct peer is
inside a configured trusted network. Caddy streams responses by default; leave response
buffering disabled for the chat event stream.

### Nginx

For Nginx on the same host as a Sprinter port bound to loopback:

```nginx
server {
    listen 443 ssl http2;
    server_name chat.example.com;

    # Configure ssl_certificate and ssl_certificate_key for your certificate.

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;
        client_max_body_size 100m;
        proxy_read_timeout 120s;
    }
}
```

For this setup, set `TRUSTED_PROXIES=127.0.0.1/32` (and `::1/128` if applicable).
Sprinter ignores `X-Forwarded-For` when the direct peer is not trusted. Keep
`proxy_buffering off` so streamed replies reach the browser promptly. The proxy body
limit must be at least as large as the configured upload limit; the default PDF limit
is 50 MB.

## Configuration

Set these environment variables on the container:

| Variable | Default | Purpose |
|---|---|---|
| `SPRINTER_PASSWORD` | Required | Master password used for login and key encryption. |
| `DATA_DIR` | `/var/lib/sprinter` | Persistent database, uploads, and logs. |
| `BIND` | `0.0.0.0` | Address the server listens on. |
| `PORT` | `8080` | HTTP port inside the container. |
| `TRUSTED_PROXIES` | Empty | Comma-separated proxy IPs or CIDRs allowed to supply forwarded client IPs. |
| `WORKER_THREADS` | `2` | Tokio worker thread count. |
| `RUST_LOG` | `info` | Log filter, for example `info,sprinter::generation=debug`. |
| `LOG_KEEP_DAYS` | `30` | Number of daily log files to retain. |
| `SPRINTER_INSECURE_COOKIES` | `false` | Development setting to allow cookies over plain HTTP. |
| `OPENROUTER_BASE_URL` | `https://openrouter.ai/api/v1` | OpenRouter API base URL; override for local testing. |

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
