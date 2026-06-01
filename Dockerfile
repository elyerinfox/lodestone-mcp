# Build the server (the headless browser is always compiled in).
FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
# build.rs orchestrates an `npm run generate` of the Nuxt dashboard
# and then `include_dir!()` embeds the static output into the binary.
# Skip that step in the Docker image — we don't ship Node here, and
# the build script handles the missing-frontend case by writing a
# "dashboard not built" page at runtime.
ENV LODESTONE_SKIP_FRONTEND=1
# include_str!() pulls in docs/instructions.md and migrations/*.sql at compile
# time -- they have to be present in the build context for the binary to
# embed them. Once the binary is built they're baked in and not needed at
# runtime; the runtime image below doesn't copy them.
COPY docs ./docs
COPY migrations ./migrations
# include_dir!() expects frontend/.output/public to exist at compile
# time; build.rs creates it empty when LODESTONE_SKIP_FRONTEND is set,
# but only if `frontend/` itself exists. Make the directory so the
# embed succeeds without dragging the whole frontend tree into the
# image.
RUN mkdir -p frontend/.output/public
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
