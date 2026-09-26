# Sprinter

Sprinter is a self-hosted AI chat application. It serves the web app and API from one
container and stores its SQLite database, uploads, backups, and logs under `DATA_DIR`.
OpenRouter is used for model discovery and chat completions; the API key is encrypted
before it is stored.

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
| `DATA_DIR` | `/var/lib/sprinter` | Persistent database, uploads, snapshots, and logs. |
| `BIND` | `0.0.0.0` | Address the server listens on. |
| `PORT` | `8080` | HTTP port inside the container. |
| `TRUSTED_PROXIES` | Empty | Comma-separated proxy IPs or CIDRs allowed to supply forwarded client IPs. |
| `BACKUP_KEEP` | `7` | Number of daily SQLite snapshots to retain. |
| `WORKER_THREADS` | `2` | Tokio worker thread count. |
| `RUST_LOG` | `info` | Log filter, for example `info,sprinter::generation=debug`. |
| `LOG_KEEP_DAYS` | `30` | Number of daily log files to retain. |
| `SPRINTER_INSECURE_COOKIES` | `false` | Development setting to allow cookies over plain HTTP. |
| `OPENROUTER_BASE_URL` | `https://openrouter.ai/api/v1` | OpenRouter API base URL; override for local testing. |

The OpenRouter key is managed in Settings. A balance is shown only when OpenRouter
returns a per-key `limit_remaining` value; keys without a spend limit show balance as
unavailable.

## Backups and restore

Sprinter writes a consistent SQLite snapshot at 03:00 UTC each day and after graceful
shutdown. It uses `VACUUM INTO`, writes a temporary file, atomically renames it into
`DATA_DIR/backups/`, and keeps the newest `BACKUP_KEEP` snapshots. The backup directory
and snapshot files are restricted to the Sprinter user.

Back up both `backups/` and `uploads/` from the persistent volume. The live
`sprinter.db`, `sprinter.db-wal`, and `sprinter.db-shm` files do not need to be copied;
logs are not part of the recommended backup set.

To restore a snapshot from the host, stop Sprinter and copy it over the live database.
For a named Docker volume, with the snapshot in the current directory:

```sh
docker stop sprinter
docker run --rm \
  -v sprinter-data:/data \
  -v "$PWD:/restore:ro" \
  alpine:3.21 \
  sh -c 'cp /restore/sprinter-YYYY-MM-DD.db /data/sprinter.db && rm -f /data/sprinter.db-wal /data/sprinter.db-shm'
docker start sprinter
```

Replace `sprinter-YYYY-MM-DD.db` with the snapshot you want. Keep the matching
`uploads/` backup alongside the database snapshot when restoring attachments.

## Operations

The container health check runs `/sprinter healthcheck`, which opens the configured
database and verifies it is reachable. Logs are written to stdout and to daily files in
`DATA_DIR/logs/`. Those files can contain short previews of chat messages and should be
protected like the database; see [design/13-logging.md](design/13-logging.md).

Releases tagged `vX.Y.Z` are built for `linux/amd64` and `linux/arm64` and published to
`ghcr.io/cau777/sprinter` with version and `latest` tags.
