# Dependencies

Everything the project pulls in, grouped by what it's there for. The
authoritative lists are [`Cargo.toml`](../Cargo.toml) and
[`frontend/package.json`](../frontend/package.json); this doc is the
prose tour so you can understand *why* each thing is there before
auditing or trimming.

## Build-time vs runtime

| Layer | Always needed | Optional |
| --- | --- | --- |
| **Build** | Rust toolchain (stable, edition 2021) | Node + npm (for the dashboard SPA — skip with `LODESTONE_SKIP_FRONTEND=1`) |
| **Runtime** | nothing beyond the single `lodestone-mcp` binary | Chrome/Chromium (for `render_page`, `html_render`, the `google` engine, `browser_*` tools); SQLite is bundled via `sqlx`; optional Linux packages for the `pdf-extract` crate (see below) |

Default-on subsystems that need *runtime* infrastructure to actually
*work* (vs. just compile): the headless browser family. Default-off
ones don't pull anything new from the host (filesystem, shell, docker,
kubernetes, databases — all gated by their `[<family>].enabled` flag).

## System dependencies

### Required to build
- **Rust toolchain (stable)** — install via [rustup](https://rustup.rs).
  Edition 2021. The project does not pin a minimum supported Rust
  version; whatever's current works. Build is straight `cargo build`
  with no nightly features.

### Required to run the headless-browser family
- **Chrome or Chromium.** Used by `render_page`, `html_render`, the
  `google` search engine, and every `browser_*` tool. Auto-detected
  via `which chromium` / common Windows install paths; override with
  `[google].chrome_path` or `LODESTONE_CHROME_PATH=…`. Inside a
  container you also need `--no-sandbox`, controlled by
  `[google].no_sandbox = true` or `LODESTONE_CHROME_NO_SANDBOX=1`.
  If Chrome isn't present, the headless paths fail with a clear
  "browser unavailable" error and everything else keeps working.

### Optional, only for `read_pdf`
- **`pdf-extract`** (a Rust crate, not a system dep) does its own
  parsing. No external `pdftotext` needed.

### Optional, only for `system_gpu`
- **NVIDIA Management Library** (NVML, ships with the NVIDIA driver).
  Only required if you want GPU info on machines with NVIDIA cards.
  Absent NVML, the tool returns "no NVIDIA GPU detected" cleanly.

### Required to build the dashboard SPA
- **Node + npm.** Tested on Node 22. The Nuxt 3 build needs no system
  packages beyond what npm pulls. Skip the dashboard build entirely
  with `LODESTONE_SKIP_FRONTEND=1` — the route serves a "not built"
  page and the binary still works for MCP clients.

## Rust crates

Categorized by purpose. The full pinned list is in
[`Cargo.toml`](../Cargo.toml); versions there are the source of truth.

### Async runtime + HTTP

- **`tokio`** — async runtime with the multi-thread scheduler.
- **`tokio-util`** — `CancellationToken` for graceful shutdown.
- **`axum`** (`features = ["ws"]`) — HTTP + WebSocket framework for
  `/mcp`, `/health`, `/ws/status`, `/api/settings/*`,
  `/constellation/*`, `/dashboard/*`.
- **`reqwest`** (`features = ["gzip", "brotli", "json", "query",
  "form", "socks"]`) — outbound HTTP client used by every provider
  and skill that hits the web.
- **`futures`** — `BoxFuture` + combinators for the skill trait.
- **`async-trait`** — object-safe `async fn` in the `SearchProvider`
  trait.

### MCP

- **`rmcp`** — Anthropic's Rust MCP SDK. Provides the `#[tool_router]`
  macro, the streamable-HTTP transport, the `Tool` model.

### Serialization + schemas

- **`serde`** + **`serde_json`** — every wire format.
- **`schemars`** — derives JSON schemas for tool argument structs;
  `Skill::schema()` returns the result.
- **`toml`** — config file parsing.
- **`serde_yaml`** — `k8s_apply` reads YAML manifests.

### Search / parsing

- **`scraper`** — CSS-selector HTML parsing for the search engines and
  StackOverflow scrapers.
- **`html2text`** — render HTML to plain text for `fetch_page`.
- **`roxmltree`** — XML parsing (used by `arxiv`, `news_feed`, RFC
  index lookups).
- **`regex`** — pattern matching across many skills.
- **`url`** — URL normalization (canonical-query keys, peer URL dedup).

### Headless browser

- **`chromiumoxide`** (`default-features = false, features =
  ["tokio-runtime"]`) — Chrome DevTools Protocol client. Powers the
  shared singleton browser + every `browser_*` tool + the rendering
  path on Google / StackOverflow.

### Embedding the dashboard

- **`include_dir`** — at compile time, embeds
  `frontend/.output/public/` into the binary as static files served
  by the `/dashboard/*` route. `build.rs` is the script that runs
  Nuxt before this fires.

### Logging

- **`tracing`** + **`tracing-subscriber`** (`features = ["env-filter",
  "fmt"]`). The reload-handle is set up by the `tracing_control`
  module so `POST /api/settings/server { log_level }` can change the
  level at runtime.

### Constellation

- **`mdns-sd`** — LAN auto-discovery (`_lodestone._tcp.local.`).

### Tool-specific clients
(Each crate maps cleanly to one or two tools — gateable via config.)

- **`bollard`** — Docker daemon socket. `docker_*` tools.
- **`kube`** + **`k8s-openapi`** — Kubernetes API. `k8s_*` tools.
- **`redis`** — Redis client. `redis_command`.
- **`sqlx`** (`features = ["postgres", "mysql", "sqlite", "chrono",
  "macros"]`) — `db_query` for Postgres/MySQL/SQLite + the
  `[memory]` SQLite store.
- **`sysinfo`** — `system_info`, `system_disks`, `system_os_release`.
- **`nvml-wrapper`** — `system_gpu` on NVIDIA boxes.
- **`machine-uid`** — stable per-host constellation node id.
- **`chrono`** + **`chrono-tz`** — `datetime`, `date_diff`,
  `time_convert`, timestamps everywhere.
- **`pdf-extract`** — `read_pdf`.
- **`kamadak-exif`** — `image_exif`.
- **`object`** — `binary_info`. ELF/PE/Mach-O.
- **`iced-x86`** — `disasm_x86_*`. x86-64 disassembly.
- **`pcap-file`** — `pcap_info`, `pcap_packets`.
- **`hound`** — `wave_info`, `wave_samples` (WAV decode).
- **`rustfft`** — `signal_fft` + spectral helpers.
- **`csv`** + **`calamine`** + **`rust_xlsxwriter`** — spreadsheet
  read/query/write.
- **`meval`** — `arithmetic_eval` expression parser.
- **`shell-words`** — POSIX-style argument splitting for `shell_run`.
- **`sgp4`** — `sat_position` / `sat_observe` orbit propagation.
- **`serialport`** — `serial_*` tools.
- **`tar`** + **`bytes`** — file-store packing / blob transit.
- **`base64`** — encodes `browser_screenshot` PNG output for the wire.

### Crypto / utility

The project deliberately uses no crypto crate for password hashing,
JWT signing, or similar. The only secret comparisons are bearer
tokens, done with the constant-time `util::ct_eq` (a hand-written
loop). FNV-1a powers the constellation key hashing — no SHA-2/3 / HMAC.

## Frontend (Nuxt 3 dashboard)

The full list is in [`frontend/package.json`](../frontend/package.json).

### Build / dev framework
- **`nuxt`** — Vue 3 meta-framework. Used in **SPA mode** (`ssr: false`)
  with static generation (`nuxt generate`) so the output is plain
  static assets the Rust binary can embed via `include_dir!`.
- **`vue`** + **`vue-router`** — the underlying view layer.
- **`typescript`** — every page and composable is TS.
- **`@types/node`** — Node typings for the build scripts.

### UI
- **`@nuxtjs/tailwindcss`** — Tailwind 3, configured in
  [`frontend/tailwind.config.ts`](../frontend/tailwind.config.ts) with
  a dark Grafana-ish palette.

### Vue composables
- **`@vueuse/core`** + **`@vueuse/nuxt`** — `useWebSocket` for the
  `/ws/status` feed with auto-reconnect; misc reactivity helpers.

That's it. No charting library, no toast library, no state-management
plugin — the SPA is intentionally small.

## Why no `rand`, `hex`, `uuid`, etc.

Several places want random IDs (constellation node id, browser
session id, guard tokens). All of them go through the same pattern:
hash `(nanos, pid, monotonic counter)` with FNV-1a and take the first
N hex chars. This means:

- No `rand` (small dep but pulls a few transitive ones).
- No `hex` (`hash_key` already returns hex).
- No `uuid` (we don't need standards-compliant UUIDs).

Trade-off: collisions aren't cryptographically protected. None of the
ID surfaces require that — collision risk vs. eight concurrent
sessions is negligible, and a stolen session id can't escalate
because every surface has its own auth gate.

## License compatibility

Lodestone is **MIT-licensed**. All listed crates and npm packages are
MIT / Apache-2.0 / BSD-style permissive. If you add a dependency, hold
to that — copyleft (GPL/AGPL) crates are out, because they would
require relicensing.

The binary embeds the dashboard SPA (`include_dir!`); the SPA depends
only on permissive packages too. If you change that — say, by adding a
GPL-only Vue plugin — the resulting binary inherits the GPL
constraint and we'd have to relicense or split the build.

## Auditing tips

- `cargo tree` — full transitive graph.
- `cargo audit` — known security advisories (install with `cargo
  install cargo-audit`).
- `cargo deny check` (with [`cargo-deny`](https://embarkstudios.github.io/cargo-deny/))
  — license / advisory / source policy enforcement. Not wired into CI
  yet but the configuration would go in `deny.toml`.
- `npm audit --prefix frontend` — frontend advisories.
- `npm ls --prefix frontend` — frontend dependency tree.

See also [`docs/security.md`](security.md) for the runtime security
posture (auth, secret handling, SSRF guard, browser sandbox, etc.) —
distinct concern from the supply-chain audit above.
