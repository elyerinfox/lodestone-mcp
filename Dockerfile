# Build the server (the headless browser is always compiled in).
FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

# Runtime image with Chromium for headless rendering.
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
