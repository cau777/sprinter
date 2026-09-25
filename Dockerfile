# Build the SPA and a statically linked Rust server in native Alpine builders. Buildx
# runs these stages for each requested platform, so rustc targets the matching CPU.
FROM node:22-alpine AS web-build
WORKDIR /src
COPY package.json package-lock.json ./
COPY web/package.json ./web/package.json
COPY e2e/package.json ./e2e/package.json
RUN npm ci
COPY web ./web
RUN npm --workspace web run build

FROM rust:1.90-alpine AS server-build
RUN apk add --no-cache build-base perl pkgconf
WORKDIR /src
COPY Cargo.toml Cargo.lock* ./
COPY crates ./crates
COPY --from=web-build /src/web/dist ./web/dist
RUN cargo build --locked --release -p sprinter

FROM alpine:3.21 AS certs
RUN apk add --no-cache ca-certificates

FROM alpine:3.21 AS runtime-fs
RUN mkdir -p /var/lib/sprinter && chown 10001:10001 /var/lib/sprinter && chmod 0700 /var/lib/sprinter

FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=runtime-fs --chown=10001:10001 /var/lib/sprinter /var/lib/sprinter
COPY --from=server-build /src/target/release/sprinter /sprinter
USER 10001:10001
EXPOSE 8080
VOLUME ["/var/lib/sprinter"]
ENV BIND=0.0.0.0 PORT=8080 DATA_DIR=/var/lib/sprinter
ENTRYPOINT ["/sprinter"]
HEALTHCHECK --interval=30s --timeout=3s --retries=3 CMD ["/sprinter", "healthcheck"]
