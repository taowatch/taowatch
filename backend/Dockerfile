# syntax=docker/dockerfile:1
# One image for both binaries: `api` (default) and `indexer run`.
FROM rust:1.100.0-trixie AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked --bin api --bin indexer \
 && cp target/release/api target/release/indexer /usr/local/bin/

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 taowatch
COPY --from=build /usr/local/bin/api /usr/local/bin/indexer /usr/local/bin/
COPY --chmod=755 <<'EOF' /usr/local/bin/healthcheck
#!/bin/sh
exec curl -fsS http://127.0.0.1:8080/api/health >/dev/null
EOF
USER taowatch
EXPOSE 8080
CMD ["api"]
