# Building lodestone-mcp

**Two artifacts, decoupled.** The MCP server (`lodestone-mcp`) is a Rust
binary that exposes `/mcp`, `/ws/status`, `/api/settings/*`,
`/constellation/*`, `/api/memory/graph`, and `/health` — no Node toolchain
anywhere in its build. The Nuxt dashboard is a **separate service**: its
own image, its own container, served by nginx, consuming the MCP
endpoints across origins. If you're driving lodestone from an MCP client
(Claude Desktop, LM Studio, etc.) you only need the MCP binary.

Four paths in increasing order of how much you want set up:

1. [Backend only](#1-backend-only) — Rust toolchain.
2. [Backend + dashboard (Docker compose)](#2-backend--dashboard-docker-compose)
   — both services side-by-side, one command.
3. [Dashboard image standalone](#3-dashboard-image-standalone) — point a
   built dashboard at an MCP server running elsewhere.
4. [Dev workflow](#4-dev-workflow) — `cargo run` for the backend, Nuxt HMR
   for the frontend.

See [`docs/dependencies.md`](dependencies.md) for the full deps list and
why each one is there.

## 1. Backend only

```sh
git clone https://github.com/elyerinfox/lodestone-mcp.git
cd lodestone-mcp
make build-release          # or: cargo build --release
```

The Rust build never touches Node. The binary lands at
`target/release/lodestone-mcp`. Run it:

```sh
./target/release/lodestone-mcp
# → MCP server on http://127.0.0.1:8000/mcp
```

Every MCP tool works. There is no `/dashboard/` route on the binary —
that was removed when the dashboard moved into its own container.

### Verify
```sh
./target/release/lodestone-mcp &
curl -s http://127.0.0.1:8000/health      # → ok
```

## 2. Backend + dashboard (Docker compose)

The shipped compose file runs the MCP server and the dashboard as **two
separate services**. Separate Dockerfiles, separate images, separate
ports — independently buildable, independently upgradable.

```sh
docker compose up --build
# or
make compose-up
```

| Service | Image | Built from | Port | What it serves |
| --- | --- | --- | --- | --- |
| `lodestone` | `lodestone-mcp` | `./Dockerfile` (Rust + Chromium) | `8000` | MCP endpoint, WS feed, settings API, constellation endpoints, memory graph API. |
| `dashboard` | `lodestone-dashboard` | `./frontend/Dockerfile` (nginx + Nuxt SPA) | `8001` | The dashboard SPA, served at `/`. Talks to the MCP service's `/ws/status` + `/api/*` from the BROWSER via the build-time `NUXT_PUBLIC_WS_URL` arg. |

The two services don't depend on each other at the application layer —
the dashboard talks to the MCP server **from your browser**, not from
inside the dashboard container. nginx just ships the static SPA.
`depends_on` is only for the MCP service's healthcheck so the dashboard
doesn't start before the MCP endpoint is reachable.

### Just the MCP server

```sh
docker compose up --build lodestone
# or:
docker build -t lodestone-mcp .
docker run --rm -p 8000:8000 lodestone-mcp
```

The MCP endpoints (`/mcp`, `/ws/status`, `/api/settings/*`,
`/constellation/*`) all work. No SPA is served — point any MCP client at
`http://localhost:8000/mcp`.

### Mounting a custom config

```sh
docker run --rm -p 8000:8000 \
  -v "$PWD/lodestone.toml:/app/lodestone.toml" \
  lodestone-mcp
```

## 3. Dashboard image standalone

Useful when the MCP server is running elsewhere (a remote host, a
different compose stack). Build the image, run it pointing at the MCP
WebSocket:

```sh
make dashboard-image
docker run --rm -p 3000:80 lodestone-dashboard
```

The standalone image bakes its WebSocket target at build time via
`--build-arg NUXT_PUBLIC_WS_URL=...`; the default points at
`ws://localhost:8000/ws/status`. Override it with:

```sh
docker build \
  --build-arg NUXT_PUBLIC_WS_URL=wss://mcp.example.com/ws/status \
  -t lodestone-dashboard -f frontend/Dockerfile frontend
```

### Pointing the compose dashboard at a different MCP

Edit `docker-compose.yml`'s `dashboard.build.args.NUXT_PUBLIC_WS_URL`,
then `docker compose build dashboard`. The SPA bakes the URL in at
`nuxt generate` time, so a rebuild is required for the change to take
effect.

## 4. Dev workflow

Two terminals, separate processes, HMR on the frontend.

### Terminal A — backend
```sh
cargo run --bin lodestone-mcp
```
Listens on `127.0.0.1:8000/mcp` + `/ws/status` + `/api/*`. Hot-restart
with `cargo watch -x run` (install with `cargo install cargo-watch`).

### Terminal B — frontend HMR
```sh
cd frontend
npm install
npm run dev
```
Opens `http://localhost:3000` with hot module reload. The dashboard HMR
build points its WebSocket at `http://localhost:8000/ws/status` via
`NUXT_PUBLIC_WS_URL`:

```sh
NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status npm run dev
```

For production-mode preview (matches what nginx serves in the dashboard
container):

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
  true` or `LODESTONE_CHROME_NO_SANDBOX=1`. The shipped Dockerfile sets
  this for you.

## Common issues

### `npm: command not found`
Install Node 22 from [nodejs.org](https://nodejs.org) or your distro's
package manager. Only needed if you're working on the dashboard; the
MCP binary doesn't need Node.

### Chrome/Chromium not detected at runtime
- macOS: install with `brew install --cask chromium`.
- Linux: `apt install chromium` (Debian/Ubuntu), `dnf install chromium`
  (Fedora), `pacman -S chromium` (Arch).
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
curl -s http://127.0.0.1:8000/dashboard/       # → 404 (no embedded dashboard — that's correct)

# fmt + clippy (CI parity)
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The integration tests behind `#[ignore]` hit live external services and
are skipped by default; run them with `cargo test -- --include-ignored`
if you have network and want full coverage.

## Makefile cheat-sheet

| Target | What |
| --- | --- |
| `make check` | `cargo fmt --all && cargo build && cargo clippy --all-targets -- -D warnings && cargo test` — the pre-commit triad. |
| `make ci` | What CI runs: `fmt --check` (not `fmt`) + clippy + build + test. |
| `make build` / `make build-release` | `cargo build` / `cargo build --release`. Never touches Node. |
| `make frontend` | Build the dashboard SPA with **host Node** into `frontend/.output/public/`. |
| `make frontend-docker` | Build the dashboard inside `node:22-bookworm` with the repo bind-mounted — no host Node required. Output lands on the host. |
| `make frontend-clean` | `rm -rf frontend/.output frontend/node_modules`. |
| `make dashboard-image` | Build the standalone `lodestone-dashboard` container image. |
| `make dashboard-run` | Run the standalone dashboard image on host port 3000. |
| `make compose-up` / `make compose-down` | Bring the two-service stack up / down. |
| `make docker` | `docker build` the MCP image + run the `/health` smoke test. |
| `make run` | `cargo run --bin lodestone-mcp` (debug profile). |

Override on the command line: `make frontend-docker
NODE_IMAGE=node:22.10-bookworm`, `make docker
IMAGE=myorg/lodestone-mcp:dev`, etc.

## See also

- [`docs/dependencies.md`](dependencies.md) — every crate and npm
  package, by purpose. License notes and audit tips.
- [`docs/configuration.md`](configuration.md) — `LODESTONE_*` env
  overrides, `lodestone.toml`, the precedence rules.
- [`docs/frontend.md`](frontend.md) — frontend architecture, page
  layout, settings drawers, WS shape.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — module map and "how to add
  a skill or provider."
