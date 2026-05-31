# Dashboard frontend

The `develop/frontend` branch adds a Nuxt 3 + Vue 3 + Tailwind dashboard
that reads live snapshots from a running `lodestone-mcp` server over
WebSockets. It is a **separate workstream** — the main `lodestone-mcp`
binary still ships and runs without the frontend; the dashboard is an
optional companion piece for operators who want a visual surface on
top of the `features` / `constellation_status` / `memory_*` tools that
already exist as MCP tools.

## Branch model

- **`main`** is the canonical history — backend changes land here.
- **`develop/frontend`** branches off main; rebased onto main
  periodically. Until the dashboard is feature-complete (memory row
  browser, constellation graph, action verbs), `develop/frontend` stays
  separate so main's release cadence isn't blocked.
- When the dashboard reaches a shippable point, `develop/frontend`
  rebases cleanly onto main and merges.

## Backend contract — `/ws/status`

The Rust server adds one route, `/ws/status`, that upgrades to a
WebSocket and pushes JSON snapshots.

### Authentication

Same `[network].token` gate as the constellation endpoints. The token is
passed as `?token=…` on connect (the browser's `WebSocket` constructor
can't set custom headers, so query-string is the only sane channel).
When `[network].token` is empty the endpoint is open. The constant-time
[`util::ct_eq`](../src/util.rs) comparator handles the check — same
guarantee as every other token-gated route in the codebase.

### Message envelope

```ts
type WsMessage = { type: 'snapshot'; data: Snapshot }
```

Tagged enum (`#[serde(tag = "type", content = "data")]` on the Rust
side). Future variants (`memo_added`, `peer_dropped`, …) slot in
without breaking older clients — they pattern-match on `type` and
ignore variants they don't recognize.

### Snapshot shape

```ts
interface Snapshot {
  server: ServerStatus       // version / uptime / tool counts / provider list
  memory: MemoryStats        // COUNT(*) per memory table (zero when disabled)
  constellation: ConstellationState  // identity / peers / delegation / seeds
}
```

Full type definitions in [`frontend/types/ws.ts`](../frontend/types/ws.ts) —
kept in lockstep by hand with [`src/ws.rs`](../src/ws.rs). The three
structs change rarely; when a field lands on the Rust side, mirror it
in the TypeScript file and the dashboard binds it automatically.

### Cadence

The server pushes one snapshot on connect, then a fresh snapshot every
[`ws::PUSH_INTERVAL`](../src/ws.rs) (5 s default). Short enough that the
dashboard feels live; long enough that it doesn't thrash a busy server.

### Privacy (rule 11)

The snapshot carries **no secrets** and **no user content**:

- Server: `<set>` / `<unset>` redaction for keys, same convention as
  `features`.
- Memory: `COUNT(*)` per table — never row bodies, never query text,
  never embedding vectors.
- Constellation: peer URLs + node ids + reputation + reachability flags
  — **never** the cluster `[network].token`, never the request body of
  any cached entry, never any peer auth material.

Adding a new field to a snapshot? Re-read [golden rule 11][rule11] and
check that what you're surfacing is the same "OK to publish in a
dashboard tile" category — counts and identifiers, not bodies.

[rule11]: golden-rules.md

## Frontend layout

```
frontend/
  app.vue                          root: NuxtLayout + NuxtPage
  layouts/default.vue              left-side nav rail + status bar +
                                   owns the WebSocket connection
  composables/
    useDashboardFeed.ts            WebSocket lifecycle + reconnect
  types/ws.ts                      typed envelope mirror of src/ws.rs
  pages/
    index.vue                      Overview — headline stats
    memory.vue                     Memory store counts
    constellation.vue              Identity + peer table + delegation
                                   knobs + seed accounting
  components/
    StatCard.vue                   big-number tile
    SectionHeading.vue             uppercase section header
  nuxt.config.ts                   Tailwind, runtime config (ws URL +
                                   token)
  tailwind.config.ts               dark / Grafana-ish palette
  tsconfig.json                    extends .nuxt/tsconfig.json
  README.md                        quick start + env vars
```

The layout owns the single WebSocket connection and `provide()`s the
reactive snapshot down to all child pages. Each page `inject()`s the
feed and binds the parts of the snapshot it cares about. Adding a
fourth page = new `pages/<name>.vue` + a new entry in the nav-items
list in `layouts/default.vue`.

## How to access it

The dashboard is **served by the `lodestone-mcp` binary itself** — no
separate Node runtime in production. After `cargo build`:

```sh
cargo run
# then open http://localhost:8000/    →   redirects to /dashboard/
# or directly:                http://localhost:8000/dashboard/
```

`build.rs` runs `npm install && npm run generate` in the `frontend/`
directory and `include_dir!()` embeds the static output
(`frontend/.output/public/`) into the binary at compile time. The
dashboard's same-origin default lets the WebSocket find `/ws/status` on
the same host:port automatically.

### Routes the binary exposes

| Path | Purpose |
| --- | --- |
| `/` | 308 redirect to `/dashboard/`. |
| `/dashboard/` | Nuxt SPA index. |
| `/dashboard/{*path}` | Static asset under the embedded SPA, or SPA fallback for client-side routes. |
| `/ws/status` | Dashboard push feed. Auth via `?token=…`. |
| `/health` | Liveness probe — `ok`. |
| `/mcp` | MCP Streamable HTTP endpoint (already existed). |
| `/constellation/*` | Peer endpoints (already existed). |

### Dev workflow (hot reload on the frontend)

The cargo build flow is fine for production-style testing, but iterating
on the Vue files is faster with the Nuxt dev server's hot reload:

```sh
# Terminal A — backend
cargo run                        # binds 0.0.0.0:8000 by default

# Terminal B — dashboard with hot reload
cd frontend
NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status npm run dev
# opens http://localhost:3000 with HMR; the WS still goes to :8000
```

When the backend has `[network].token` set:

```sh
NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status \
  NUXT_PUBLIC_WS_TOKEN=smoke-token \
  npm run dev
```

Type-check the frontend with `npm run typecheck`. Skip the
dashboard build entirely (for contributors without Node) by setting
`LODESTONE_SKIP_FRONTEND=1` in the environment before
`cargo build` — the binary then serves a small "dashboard not built"
HTML page at `/dashboard/`, and everything else works normally.

### Build flow

```
cargo build
  → build.rs
      → ensure frontend/.output/public/ exists
      → if npm on PATH AND LODESTONE_SKIP_FRONTEND not set:
          npm ci (or npm install on lockfile drift)
          npm run generate
      → write cargo:rerun-if-changed for frontend sources
  → src/ws.rs: include_dir!("frontend/.output/public")
      → embeds whatever's there (empty when build was skipped)
  → src/main.rs::dashboard_routes() serves the embedded Dir
```

`cargo:rerun-if-changed` is scoped so editing `frontend/pages/index.vue`
triggers an incremental rebuild (Nuxt + Rust link), but a normal
`cargo build` against unchanged frontend sources just relinks the
existing embed.

## What's deferred

These are deliberately out of scope for the initial scaffold:

- **Memory row browser.** The feed carries counts only (rule 11 ✓).
  Viewing individual memos / solutions needs a separate authenticated
  REST endpoint and pagination — bigger surface, more careful auth.
- **Constellation graph visualization.** The peer table is the
  load-bearing view today; a force-directed graph over `peers[*].known`
  (the gossip edges) would show the full mesh shape. Needs a chart
  library or a small custom SVG renderer.
- **Action verbs from the dashboard.** `solution_forget`,
  `conversation_prune`, `constellation_seeds_reset`, etc. The feed is
  push-only for v1; a second authenticated REST endpoint with the
  existing destructive-guard token flow would let the UI fire them.
- **Embed the production build in the binary.** `include_dir!` against
  `.output/public/` would let `lodestone-mcp` serve the dashboard
  itself, no separate Node runtime in production. For now the dev
  workflow runs the Nuxt dev server next to the backend.
