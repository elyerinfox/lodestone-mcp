# Building lodestone-mcp

**The dashboard is optional.** The MCP server itself — `/mcp`,
`/ws/status`, `/api/settings/*`, `/constellation/*` — exposes every
endpoint with no Node toolchain involved at any stage. The Nuxt
dashboard is a separate, opt-in operator view that *consumes* those
endpoints. If you're running lodestone-mcp purely as an MCP server
behind an MCP client, the dashboard is overhead you don't need.

The mechanism: the binary always serves a `/dashboard/` route, but
what it serves is determined by what's in `frontend/.output/public/`
at compile time. Empty directory → a small "dashboard not built" page
with build instructions. Populated directory (from `make frontend` or
`make frontend-docker`) → the actual SPA. Cargo's `build.rs` only
ensures the embed target exists; it never reaches into the frontend
tree. `cargo build` produces a working binary either way.

Five paths in increasing order of how much you want set up:

1. [Backend only](#1-backend-only) — Rust toolchain. Fastest. The
   dashboard route returns the "not built" page; every MCP tool works.
2. [Backend + dashboard](#2-backend--dashboard) — Rust + Node. Build
   the SPA, then build the binary; the binary embeds it.
2b. [Backend on host, frontend in Docker](#2b-backend-on-host-frontend-built-in-docker)
    — Rust on the host, no Node on the host. Frontend builds inside a
    `node:22-bookworm` container that bind-mounts your repo and writes
    `frontend/.output/public/` back to the host. Same final binary.
3. [Docker (everything in containers)](#3-docker-everything-in-containers)
    — no host toolchain at all; bundles Chromium.
4. [Dev workflow](#4-dev-workflow) — `cargo run` for the backend +
   `npm run dev` for the dashboard with HMR pointing at the
   running backend.

See [`docs/dependencies.md`](dependencies.md) for the full deps list
and why each one is there.

## 1. Backend only

```sh
git clone https://github.com/elyerinfox/lodestone-mcp.git
cd lodestone-mcp
make build-release          # or: cargo build --release
```

No env vars, no opt-out. The Rust build never touches Node:
`build.rs` only ensures the embed directory exists; whatever's there
gets baked in. With nothing in the directory, the `/dashboard/`
route serves a small "not built" page. Every MCP tool works
normally.

The binary lands at `target/release/lodestone-mcp`. Run it with
`./target/release/lodestone-mcp` — listens on `127.0.0.1:8000/mcp` by
default.

### Verify
```sh
./target/release/lodestone-mcp &
curl -s http://127.0.0.1:8000/health      # → ok
```

## 2. Backend + dashboard

Two explicit steps: build the SPA, then build the binary. The binary
embeds whatever's in `frontend/.output/public/` at compile time.

```sh
git clone https://github.com/elyerinfox/lodestone-mcp.git
cd lodestone-mcp
make build-with-dashboard
```

That target chains `make frontend` (host Node builds the SPA) and
`make build-release`. Equivalent to:

```sh
cd frontend && npm ci && npm run generate && cd ..
cargo build --release
```

Required on `PATH`:
- **Rust** (stable).
- **Node + npm** (tested on Node 22).

### Re-runs

`build.rs` declares `cargo:rerun-if-changed=frontend/.output/public`,
so editing a Vue file → `make frontend` → next `cargo build`
re-embeds the binary without a manual `cargo clean`. The build
script doesn't know about `frontend/` source files — npm runs only
when you ask for it.

To force a clean dashboard rebuild:
```sh
make frontend-clean         # rm -rf frontend/.output frontend/node_modules
make frontend               # or: make frontend-docker
make build-release
```

## 2b. Backend on host, frontend built in Docker

The Rust binary embeds the dashboard via `include_dir!()` reading
`frontend/.output/public/`. The directory lives on your filesystem
either way — the question is just *how* you populate it. If you don't
want Node on the host, shell the frontend build out to a Node
container, mount your repo into it, and let it write the output back
to your disk. Then build the binary normally.

The Makefile wraps this in one command:

```sh
make frontend-docker     # populate frontend/.output/public/ via node:22-bookworm
make build-release       # cargo build --release embeds it
# or in one shot:
make build-with-dashboard-docker
```

What `make frontend-docker` runs:
```sh
docker run --rm \
  -v "$PWD:/work" -w /work/frontend \
  node:22-bookworm sh -c "npm ci && npm run generate"
```

Result: dashboard built inside the container, output written to your
host's `frontend/.output/public/`, no Node ever installed on the
host. Then the host's `cargo build --release` picks it up and embeds
it. The resulting binary is identical to one built with host Node.

Pin the Node version by overriding `NODE_IMAGE=node:22.10-bookworm`.

If `node_modules` was last installed inside the container (root-owned
inside the bind mount), `make frontend-clean` is the easy reset —
it's `rm -rf frontend/.output frontend/node_modules`.

## 3. Docker (everything in containers)

```sh
docker compose up --build
# or
docker build -t lodestone-mcp .
docker run --rm -p 8000:8000 lodestone-mcp
```

The shipped image is **three-stage** and bundles the dashboard:

- **Stage 1 — `frontend`** — `node:22-bookworm-slim`. Runs `npm ci`
  then `npm run generate` to produce `frontend/.output/public/`.
  Docker layer caching means this stage only re-runs when files
  under `frontend/` actually change.
- **Stage 2 — `build`** — `rust:1-bookworm`. `cargo build --release`,
  copying the stage-1 SPA into `frontend/.output/public/` so
  `include_dir!()` embeds it.
- **Stage 3 — `runtime`** — `debian:bookworm-slim` plus `chromium`,
  `ca-certificates`, `fonts-liberation`. Sets `LODESTONE_BIND=
  0.0.0.0:8000`, `LODESTONE_CHROME_PATH=/usr/bin/chromium`, and
  `LODESTONE_CHROME_NO_SANDBOX=1` (root containers need that flag).

The resulting image serves both `/mcp` and the embedded dashboard at
`/dashboard/` from the same port.

### Building a dashboard-less Docker image

If you want a lighter image without the SPA — for an MCP-only
deployment — edit the Dockerfile's build stage:

```dockerfile
# Replace this line:
COPY --from=frontend /app/frontend/.output/public /app/frontend/.output/public
# with:
RUN mkdir -p frontend/.output/public
```

The binary still serves `/dashboard/` but it returns the "not built"
page. Removes the Node stage from your rebuild path entirely.

### Mounting a custom config
```sh
docker run --rm -p 8000:8000 \
  -v "$PWD/lodestone.toml:/app/lodestone.toml" \
  lodestone-mcp
```

### Building the dashboard into the Docker image

If you want the full dashboard inside the container, drop the
`LODESTONE_SKIP_FRONTEND=1` line and add a Node stage:

```dockerfile
FROM node:22-bookworm AS frontend
WORKDIR /app
COPY frontend ./frontend
RUN cd frontend && npm ci && npm run generate

FROM rust:1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY docs ./docs
COPY migrations ./migrations
COPY --from=frontend /app/frontend ./frontend
ENV LODESTONE_SKIP_FRONTEND=1   # source already generated above
RUN cargo build --release
```

The shipped Dockerfile picks the lean path because most MCP clients
don't need the dashboard.

## 4. Dev workflow

Two terminals, separate processes, HMR on the frontend.

### Terminal A — backend
```sh
LODESTONE_SKIP_FRONTEND=1 cargo run
```
Listens on `127.0.0.1:8000/mcp` + `/ws/status`. Hot-restart with
`cargo watch -x run` (install with `cargo install cargo-watch`).

### Terminal B — frontend HMR
```sh
cd frontend
npm install
npm run dev
```
Opens `http://localhost:3000` with hot module reload. The dashboard
HMR build points its WebSocket at `http://localhost:8000/ws/status`
via `NUXT_PUBLIC_WS_URL`:

```sh
NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status npm run dev
```

For production-mode preview (matches what the embedded binary
serves):
```sh
cd frontend
npm run generate
npx serve .output/public
```

## Runtime requirements

- **Chrome or Chromium** is required at runtime for `render_page`,
  `html_render`, the `google` search engine, and every `browser_*`
  tool. Auto-detected; override with `[google].chrome_path` or
  `LODESTONE_CHROME_PATH=/path/to/chromium`.
- If Chrome isn't present, the headless paths fail with a clear
  "browser unavailable" error and everything else keeps working.
- Inside containers, add `--no-sandbox`: set `[google].no_sandbox =
  true` or `LODESTONE_CHROME_NO_SANDBOX=1`. The shipped Dockerfile
  sets this for you.

## Common issues

### `error: couldn't read build.rs`
The Docker `COPY` line is missing `build.rs`. The shipped
[`Dockerfile`](../Dockerfile) copies it; if you forked an older copy,
add `build.rs` to the `COPY Cargo.toml Cargo.lock ... ./` line in the
build stage.

### `LODESTONE_SKIP_FRONTEND set — dashboard build skipped`
Not an error — `build.rs` printing a `cargo:warning`. Means the
binary won't carry the SPA. Drop the env var (and ensure Node + npm
are installed) for the full build.

### `npm: command not found`
Install Node 22 from [nodejs.org](https://nodejs.org) or your
distro's package manager. Or skip the dashboard build with
`LODESTONE_SKIP_FRONTEND=1`.

### `npm ci failed, falling back to npm install`
Not an error — `build.rs` recovering from lockfile drift. The
fallback re-resolves dependencies; the resulting build is fine but
not reproducible against the lockfile. If you want strict
reproducibility, run `cd frontend && npm ci` yourself and fix the
underlying drift.

### Chrome/Chromium not detected at runtime
- macOS: install with `brew install --cask chromium`.
- Linux: `apt install chromium` (Debian/Ubuntu), `dnf install
  chromium` (Fedora), `pacman -S chromium` (Arch).
- Windows: install Chrome from google.com/chrome; auto-detected from
  the standard install paths.
- Custom location: `LODESTONE_CHROME_PATH=/path/to/chrome
  ./lodestone-mcp` or `[google].chrome_path = "..."` in config.

### `failed to remove file lodestone-mcp.exe` (Windows)
The existing binary is locked because the previous instance is still
running. Stop it before rebuilding:
```powershell
Get-Process lodestone-mcp -ErrorAction SilentlyContinue | Stop-Process -Force
cargo build
```

### SQLite or other native dep build failure
`sqlx` uses bundled SQLite — no system SQLite required. If a build
fails with a linker error, it's almost always a missing C toolchain
(install `build-essential` on Debian / Xcode CLT on macOS / VS Build
Tools on Windows).

## Verifying the build

After a successful build:

```sh
# Backend
./target/release/lodestone-mcp &
curl -s http://127.0.0.1:8000/health           # → ok
curl -s http://127.0.0.1:8000/dashboard/       # → 200 (SPA or "not built" page)

# fmt + clippy (CI parity)
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The integration tests behind `#[ignore]` hit live external services
and are skipped by default; run them with `cargo test --
--include-ignored` if you have network and want full coverage.

## Makefile cheat-sheet

| Target | What |
| --- | --- |
| `make check` | `cargo fmt --all && cargo build && cargo clippy --all-targets -- -D warnings && cargo test` — the pre-commit triad. |
| `make ci` | What CI runs: `fmt --check` (not `fmt`) + clippy + build + test. |
| `make build` / `make build-release` | `cargo build` / `cargo build --release`. The dashboard is built only if Node is on PATH (or skipped via `LODESTONE_SKIP_FRONTEND=1`). |
| `make frontend` | Build the dashboard SPA with **host Node** into `frontend/.output/public/`. |
| `make frontend-docker` | Build the dashboard inside `node:22-bookworm` with the repo bind-mounted — no host Node required. Output lands on the host. |
| `make frontend-clean` | `rm -rf frontend/.output frontend/node_modules`. |
| `make build-with-dashboard` | `make frontend` then `make build-release`. Single command for the full embedded build. |
| `make build-with-dashboard-docker` | `make frontend-docker` then `make build-release`. Same final binary, no host Node. |
| `make docker` | `docker build` the image + run the `/health` smoke test. |
| `make run` | `cargo run` (debug profile). |

Override on the command line: `make frontend-docker NODE_IMAGE=node:22.10-bookworm`,
`make docker IMAGE=myorg/lodestone-mcp:dev`, etc.

## See also

- [`docs/dependencies.md`](dependencies.md) — every crate and npm
  package, by purpose. License notes and audit tips.
- [`docs/configuration.md`](configuration.md) — `LODESTONE_*` env
  overrides, `lodestone.toml`, the precedence rules.
- [`docs/frontend.md`](frontend.md) — frontend architecture, page
  layout, settings drawers, WS shape.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — module map and "how to
  add a skill or provider."
