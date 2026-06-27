# syntax=docker/dockerfile:1

FROM rust:1-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/Cargo.toml src/Cargo.toml
COPY src/src src/src
COPY src/tests src/tests

RUN cargo build --release -p rustrank

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --uid 10001 --user-group --create-home --home-dir /home/rustrank --shell /usr/sbin/nologin rustrank \
    && mkdir -p /workspace \
    && chown -R rustrank:rustrank /workspace /home/rustrank

COPY --from=builder /app/target/release/rustrank /usr/local/bin/rustrank

ENV RUSTRANK_TRANSPORT=streamable_http \
    RUSTRANK_HOST=0.0.0.0 \
    RUSTRANK_PORT=63477 \
    RUSTRANK_MCP_PATH=/mcp \
    RUST_LOG=info

WORKDIR /workspace
VOLUME ["/workspace"]
EXPOSE 63477

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${RUSTRANK_PORT:-63477}/healthz" >/dev/null || exit 1

USER rustrank
ENTRYPOINT ["rustrank"]
