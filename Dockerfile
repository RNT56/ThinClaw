# Multi-stage Dockerfile for the ThinClaw agent (cloud deployment).
#
# Build:
#   docker build --platform linux/amd64 -t thinclaw:latest .
#   docker build --build-arg BUILD_FEATURES=light -t thinclaw:light .
#
# Run:
#   docker run --env-file .env -p 3000:3000 thinclaw:latest

# Stage 1: Build
FROM rust:1.92-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake gcc g++ \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-wasip2 \
    && cargo install wasm-tools

WORKDIR /app
ARG BUILD_FEATURES=full

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Copy source, build script, tests, and supporting directories
COPY build.rs build.rs
COPY src/ src/
COPY tests/ tests/
COPY benches/ benches/
COPY migrations/ migrations/
COPY patches/ patches/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY tools-src/ tools-src/
COPY wit/ wit/

RUN if [ "$BUILD_FEATURES" = "default" ] || [ -z "$BUILD_FEATURES" ]; then \
        cargo build --release --bin thinclaw; \
    else \
        cargo build --release --bin thinclaw --features "$BUILD_FEATURES"; \
    fi

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/thinclaw /usr/local/bin/thinclaw
COPY --from=builder /app/migrations /app/migrations

# Non-root user
RUN useradd -m -u 1000 -s /bin/bash thinclaw
USER thinclaw

EXPOSE 3000

ENV RUST_LOG=thinclaw=info \
    GATEWAY_PORT=3000

ENTRYPOINT ["thinclaw"]
