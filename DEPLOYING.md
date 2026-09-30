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