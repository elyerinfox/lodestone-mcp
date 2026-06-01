# Standalone dashboard image — Nuxt SPA served by nginx on port 80.
#
# Use this when you want the dashboard to live in its own container,
# separate from the MCP server. Talks to the MCP server over the
# network via NUXT_PUBLIC_WS_URL (set in the runtime environment).
# Build the binary's dashboard-embedded variant instead if you'd
# rather ship a single artifact — see ../Dockerfile and the
# `make build-with-dashboard-docker` flow.
#
# Build from the repository ROOT (so the build context is `frontend/`):
#   docker build -t lodestone-dashboard -f frontend/Dockerfile frontend
#
# Or via docker-compose, which wires the WS URL through automatically.

# --- stage 1: build the SPA ---
FROM node:22-bookworm-slim AS build
WORKDIR /app

# Install deps first so an unchanged package-lock.json caches a layer.
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund

# Then the sources. Build with root baseURL so nginx serves at /;
# the default `/dashboard/` baseURL is what the embedded path uses
# when the SPA lives inside the Rust binary.
COPY . .
# The standalone SPA isn't same-origin with the MCP server, so the
# WebSocket URL has to be baked in at build time. Default points at
# the host's published port for the MCP service in our compose file.
# Override via:
#   docker build --build-arg NUXT_PUBLIC_WS_URL=ws://mcp.example.com/ws/status …
ARG NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status
ENV NUXT_APP_BASE_URL=/ \
    NUXT_PUBLIC_WS_URL=${NUXT_PUBLIC_WS_URL}
RUN npm run generate

# --- stage 2: serve via nginx ---
FROM nginx:1-alpine AS runtime

# Replace the default site with one that does SPA-style fallback
# (Vue Router HTML5 history mode needs every unknown path to fall
# through to index.html instead of 404-ing). Health endpoint at
# /health for compose / k8s probes.
RUN { \
        echo 'server {'; \
        echo '    listen 80 default_server;'; \
        echo '    listen [::]:80 default_server;'; \
        echo '    root /usr/share/nginx/html;'; \
        echo '    index index.html;'; \
        echo '    location = /health { return 200 "ok\n"; add_header Content-Type text/plain; }'; \
        echo '    location / {'; \
        echo '        try_files $uri $uri/ /index.html;'; \
        echo '    }'; \
        echo '    # Long-cache the hashed Nuxt assets.'; \
        echo '    location /_nuxt/ {'; \
        echo '        expires 30d;'; \
        echo '        add_header Cache-Control "public, immutable";'; \
        echo '    }'; \
        echo '}'; \
    } > /etc/nginx/conf.d/default.conf

COPY --from=build /app/.output/public /usr/share/nginx/html

EXPOSE 80

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD wget -qO- http://127.0.0.1/health || exit 1
