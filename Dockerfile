# Stage 1 — build the Nuxt dashboard SPA.
#
# Decoupled from the Rust build (per the host-side workflow) but still
# baked into the operator-facing Docker image, since `docker compose
# up --build` is meant to ship a full experience without further steps.
# Docker layer caching means this stage re-runs only when files under
# frontend/ change.
FROM node:22-bookworm-slim AS frontend
WORKDIR /app/frontend
# Install deps first so unchanged package-lock.json caches a layer.
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
# Then copy sources and produce the static export under .output/public.
COPY frontend/ ./
RUN npm run generate

# Stage 2 — build the Rust binary, embedding the SPA from stage 1.
FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
# include_str!() pulls in docs/instructions.md and migrations/*.sql at
# compile time — they have to be present in the build context for the
# binary to embed them. Baked in; the runtime image doesn't copy them.
COPY docs ./docs
COPY migrations ./migrations
# include_dir!() reads this path at compile time; copy the stage-1
# output here so the binary ships with the dashboard. To produce a
# dashboard-less binary, replace the COPY with
# `RUN mkdir -p frontend/.output/public` — the binary will then
# serve the small "not built" page on /dashboard at runtime.
COPY --from=frontend /app/frontend/.output/public /app/frontend/.output/public
RUN cargo build --release

# Stage 3 — runtime image with Chromium for headless rendering.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        chromium \
        ca-certificates \
        fonts-liberation \
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

# Mount a custom config with: -v ./lodestone.toml:/app/lodestone.toml
ENTRYPOINT ["lodestone-mcp"]
