# MCP server image — Rust + Chromium, no Node, no SPA.
#
# The dashboard is a SEPARATE concern with its OWN Dockerfile under
# `frontend/`. Two deployment shapes are supported:
#
#   1. MCP-only (this image): the binary serves /dashboard with a
#      small "not built" page that links to the build instructions.
#      Every MCP endpoint works.
#   2. MCP + standalone dashboard: this image plus the dashboard
#      image (frontend/Dockerfile), orchestrated by the shipped
#      docker-compose.yml. The SPA talks to /ws/status on the MCP
#      service via NUXT_PUBLIC_WS_URL.
#
# To bake the dashboard INTO the MCP binary instead of running it as a
# separate container, run `make frontend` or `make frontend-docker` on
# the host first — those populate frontend/.output/public/, and a
# subsequent `cargo build` embeds it via include_dir!(). The
# `frontend/.output/public/` directory in this build context is then
# copied in by the COPY line below (see docs/building.md).

# --- build stage: rustc + the binary ---
FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
# include_str!() pulls in docs/instructions.md and migrations/*.sql at
# compile time. Required in the build context for the binary to embed
# them; baked in afterward, not needed at runtime.
COPY docs ./docs
COPY migrations ./migrations
# include_dir!() reads this path at compile time. If frontend/.output/
# public/ exists in the build context (because you ran `make frontend`
# / `make frontend-docker` before `docker build`), its contents get
# embedded. Otherwise the script creates it empty and the dashboard
# route returns the "not built" page. .dockerignore excludes
# frontend/node_modules + frontend/.nuxt so we don't ship those.
COPY frontend ./frontend
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
