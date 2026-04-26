# Lightweight packaging Dockerfile for the ThinClaw agent (cloud deployment).
#
# This Dockerfile does NOT compile Rust from source. Instead, it packages
# a pre-built binary (produced by the release workflow's cargo-dist job)
# into a minimal Debian runtime image.
#
# Build (CI — binary injected via --build-arg):
#   docker build --build-arg THINCLAW_BINARY=./thinclaw \
#                --platform linux/amd64 -t thinclaw:latest .
#
# The old multi-stage build approach compiled Rust inside Docker, which
# caused ARM64 builds to hang indefinitely under QEMU emulation.

FROM debian:bookworm-slim

# Runtime dependencies only — no compiler toolchain
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl \
    && rm -rf /var/lib/apt/lists/*

# The pre-built binary is injected by the CI job (downloaded from artifacts)
ARG THINCLAW_BINARY=thinclaw
COPY ${THINCLAW_BINARY} /usr/local/bin/thinclaw
RUN chmod +x /usr/local/bin/thinclaw

# Copy migrations (these are SQL files, not compiled artifacts)
COPY migrations /app/migrations

# Non-root user
RUN useradd -m -u 1000 -s /bin/bash thinclaw \
    && mkdir -p /data /workspace \
    && chown -R thinclaw:thinclaw /data /workspace
USER thinclaw

EXPOSE 3000

ENV RUST_LOG=thinclaw=info \
    GATEWAY_PORT=3000

ENTRYPOINT ["thinclaw"]
