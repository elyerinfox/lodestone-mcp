# MCP server image — Rust + Chromium, no Node, no SPA.
#
# The dashboard is a SEPARATE service with its OWN Dockerfile under
# `frontend/`. The MCP binary serves only `/mcp`, `/ws/status`,
# `/api/settings/*`, `/api/memory/graph`, `/constellation/*`, `/health`
# — no `/dashboard/*` route. The full stack (MCP + dashboard) is
# orchestrated by the shipped docker-compose.yml. The SPA talks to
# `/ws/status` on the MCP service via NUXT_PUBLIC_WS_URL.

# --- build stage: rustc + the binary ---
FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
# include_str!() pulls in docs/instructions.md and migrations/*.sql at
# compile time. Required in the build context for the binary to embed
# them; baked in afterward, not needed at runtime.
COPY docs ./docs
COPY migrations ./migrations
RUN cargo build --release

# --- runtime stage: just Chromium + the binary ---
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        chromium \
        ca-certificates \
        fonts-liberation \
        wget \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /app/target/release/lodestone-mcp /usr/local/bin/lodestone-mcp
# The runtime looks for `config/` relative to its WORKDIR. Without it
# the binary falls back to compile-time defaults — which leaves the
# constellation off and, as a side effect, leaves the /api/* routes
# unmounted (they're gated on a constellation handle existing). Ship
# the same shipped baseline operators get on a host install.
COPY config /app/config
WORKDIR /app

# Defaults for running in a container:
# - bind on all interfaces so the published port is reachable from the host
# - point at the distro's Chromium (override with -e LODESTONE_CHROME_PATH=...)
# - Chrome must run with --no-sandbox as root inside the container
ENV LODESTONE_BIND=0.0.0.0:8000 \
    LODESTONE_CHROME_PATH=/usr/bin/chromium \
    LODESTONE_CHROME_NO_SANDBOX=1

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8000/health || exit 1

# Mount a custom config with: -v ./lodestone.toml:/app/lodestone.toml
ENTRYPOINT ["lodestone-mcp"]
