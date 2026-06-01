# Building lodestone-mcp

Four paths in increasing order of how much you want set up:

1. [Backend only](#1-backend-only) — Rust toolchain. Fastest. Dashboard
   route serves a "not built" page, every MCP tool works.
2. [Backend + dashboard](#2-backend--dashboard) — Rust + Node. The full
   embedded experience. One binary at the end.
3. [Docker](#3-docker) — no host toolchain needed; bundles Chromium.
4. [Dev workflow](#4-dev-workflow) — `cargo run` for the backend +
   `npm run dev` for the dashboard with HMR pointing at the
   running backend.

See [`docs/dependencies.md`](dependencies.md) for the full deps list
and why each one is there.

## 1. Backend only

```sh
git clone https://github.com/elyerinfox/lodestone-mcp.git
cd lodestone-mcp
LODESTONE_SKIP_FRONTEND=1 cargo build --release
```

`LODESTONE_SKIP_FRONTEND=1` tells [`build.rs`](../build.rs) not to
attempt the Nuxt build. The `/dashboard/*` route at runtime serves a
small "not built" page; every MCP tool works normally.

The binary lands at `target/release/lodestone-mcp`. Run it with
`./target/release/lodestone-mcp` — listens on `127.0.0.1:8000/mcp` by
default.

### Verify
```sh
./target/release/lodestone-mcp &
curl -s http://127.0.0.1:8000/health      # → ok
```

## 2. Backend + dashboard

The Rust binary embeds the static Nuxt SPA via `include_dir!()`. The
build script runs `npm run generate` first, then cargo embeds the
output. End state: one binary, dashboard served from `/dashboard/`.

```sh
git clone https://github.com/elyerinfox/lodestone-mcp.git
cd lodestone-mcp
cargo build --release
```

Required on `PATH`:
- **Rust** (stable).
- **Node + npm** (tested on Node 22).

What `build.rs` does, in order:
1. Creates `frontend/.output/public/` (always — `include_dir!()`
   needs a directory).
2. Honors `LODESTONE_SKIP_FRONTEND=1` if set, skips everything else.
3. Falls through if `frontend/package.json` is missing or `npm` is not
   on `PATH` — prints a `cargo:warning` and skips the build.
4. `npm ci` (or `npm install` if the lockfile drifted) under
   `frontend/`.
5. `npm run generate` (Nuxt's static export).
6. Cargo then `include_dir!()`s `frontend/.output/public/` into the
   binary.

### Re-runs

`build.rs` declares `cargo:rerun-if-changed` for every source path
Nuxt consumes (`package.json`, `nuxt.config.ts`, `tailwind.config.ts`,
`tsconfig.json`, `app.vue`, `types/`, `composables/`, `layouts/`,
`pages/`, `components/`), so editing a Rust file doesn't re-run npm.
Editing a Vue file does.

To force a clean dashboard rebuild:
```sh
rm -rf frontend/.output frontend/node_modules
touch build.rs
cargo build --release
```

## 3. Docker

```sh
docker compose up --build
# or
docker build -t lodestone-mcp .
docker run --rm -p 8000:8000 lodestone-mcp
```

The image is two-stage:

- **Build stage** — `rust:1-bookworm` base. Skips the dashboard
  (`LODESTONE_SKIP_FRONTEND=1` baked in) so we don't need Node in the
  image. The dashboard route at runtime serves the "not built" page.
- **Runtime stage** — `debian:bookworm-slim` plus `chromium`,
  `ca-certificates`, `fonts-liberation`. Sets `LODESTONE_BIND=
  0.0.0.0:8000`, `LODESTONE_CHROME_PATH=/usr/bin/chromium`, and
  `LODESTONE_CHROME_NO_SANDBOX=1` (root containers need that flag).

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

## See also

- [`docs/dependencies.md`](dependencies.md) — every crate and npm
  package, by purpose. License notes and audit tips.
- [`docs/configuration.md`](configuration.md) — `LODESTONE_*` env
  overrides, `lodestone.toml`, the precedence rules.
- [`docs/frontend.md`](frontend.md) — frontend architecture, page
  layout, settings drawers, WS shape.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — module map and "how to
  add a skill or provider."
