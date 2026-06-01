# Security model & audit reference

This document catalogs every security-relevant surface in lodestone-mcp:
where the trust boundaries sit, what each control enforces, what code
backs it, and what follow-up work is still open. It's organized so an
auditor can walk top-to-bottom; every section ends with a "where it
lives" pointer to the source file(s) that implement the control.

For the underlying invariants this document references, see
[`golden-rules.md`](golden-rules.md) — those are the project's hard
constraints. This file is the audit-oriented surface that explains how
each rule is realized in code.

## Trust boundaries

lodestone-mcp talks to four classes of caller. The privilege each one
gets is intentionally different:

| Caller                  | Trust level | What it can touch                                              | Auth                                                     |
| ----------------------- | ----------- | -------------------------------------------------------------- | -------------------------------------------------------- |
| Local MCP client        | High        | Every enabled tool, full skill router.                         | `auth_token` (optional bearer; default empty = open).    |
| Dashboard browser       | High        | WS feed + `/api/settings/*` ephemeral knobs.                   | `[network].token` (shared with constellation).           |
| Constellation peers     | Low         | `/constellation/*` only — digest, query, blob, retrieve.       | `[network].token` (constant-time check).                 |
| Galaxy broker (if any)  | Low         | `/galaxy/*` directory lookups; never proxies traffic.          | `[galaxy].token`.                                        |

The MCP port (`bind`) and the constellation port (`[network].bind`) are
**separate listeners** by default — MCP stays on `127.0.0.1`; the
constellation binds `0.0.0.0` so peers can reach it without exposing
the MCP endpoint to the LAN. See `src/main.rs::main` for where the two
routers are mounted.

## Authentication

### MCP bearer (`auth_token`)
Gates the MCP endpoint. Optional. Empty value = no auth (the default,
matched to local-loopback bind). Compared with [`util::ct_eq`](../src/util.rs)
(constant-time) so a wrong token doesn't leak its length or prefix via
timing.

### Constellation token (`[network].token`)
Gates `/constellation/*` AND `/ws/status` AND `/api/settings/*`. Same
constant-time check via `Constellation::token_ok`. The dashboard's
WebSocket passes it as `?token=...` (browsers can't set custom headers
on `WebSocket`); the settings endpoints accept it as
`Authorization: Bearer …`.

> **Where**: `src/constellation/mod.rs::token_ok`,
> `src/main.rs::api_routes::presented_token`,
> `src/main.rs::ws_routes::handler`.

### Galaxy token (`[galaxy].token`)
Gates the participating side's calls into a broker (and the broker's
own endpoint authentication). Galaxy is opt-in only; default config
disables it.

> **Where**: `src/galaxy/broker.rs`.

### Per-tool rate-limit on delegation
Even when a peer authenticates, delegated retrieval is rate-limited per
peer per hour (`[network].delegation_*` knobs) by
`src/constellation/delegation.rs`. Reasoning: a stolen/compromised
constellation token can't drain your bandwidth at line rate.

## Secret handling (golden rule 11)

Credentials never appear in:

- **Logs.** `tracing` calls referencing secret-shaped config fields
  use the `<set>` / `<unset>` redaction, not the value.
- **Tool responses.** The `features` tool is the load-bearing example
  — it surfaces `[github].token`, `[eia].key`, `[network].token`, etc.
  as `<set>` / `<unset>`.
- **Disk under `config/`.** The shipped baseline contains no real
  credentials. Per-host overrides live in `lodestone.toml`, which is
  gitignored.
- **The WS feed.** The dashboard's per-secret presence panel
  (`SecretRow.vue`) only ever sees booleans — `ServerStatus.secrets`
  is `{auth_token: bool, network_token: bool, …}` with no value
  field. `src/ws.rs::SecretPresence`.
- **Constellation gossip.** Digests carry only HASHES of normalized
  query keys; result payloads that traverse a peer consult are public
  web data (keyless providers only — golden rule 3).
- **The settings drawers.** The "Hide secret fields entirely" policy
  is enforced at the UI: no panel renders or accepts a secret. See
  the "Secrets policy" decision in the dashboard.

Constant-time comparison via `ct_eq` is mandatory for any bearer-token
check; `==` on secret bytes leaks length+prefix via early exit.

> **Where**: `src/util.rs::ct_eq`,
> `src/skills/meta.rs` (the `features` tool's redaction loop),
> `src/ws.rs::ServerStatus`.

## Destructive-action guards (golden rule 8)

Every tool that deletes, overwrites, or executes arbitrary code goes
through ONE of:

1. **Family disabled** (`[<family>].enabled = false`) — the tool isn't
   exposed at all.
2. **MCP elicitation** (where the client supports it) — interactive
   confirm.
3. **Guard challenge** (the default, client-agnostic):
   - First call performs nothing, returns a one-time `confirm` token.
   - Second call with `confirm=<token>` executes.
   - `trust=true` whitelists the action for the rest of the session.
   - Per-family `allow_destructive` flag pre-authorizes (skip the
     prompt).

Tools that route through the guard include `fs_delete`, `fs_move`,
`fs_write`, `fs_edit`, `fs_mkdir`, `docker_stop|remove|exec|rmi`,
`k8s_delete`, write-mode `db_query` / `redis_command`, `shell_run`,
`python_run`, `ffmpeg_convert`, `sheet_write`, `systemd_*`,
`memory_forget`, `solution_forget`, every git destructive subcommand.

> **Where**: `src/skills/guard.rs`, plus the call-site invocations in
> each guarded skill.

## Path traversal

The filesystem skills only act on paths that resolve under
`[filesystem].roots` — `filesystem::resolve` canonicalizes the input
and rejects anything that climbs out via `..`. Every read-a-file
helper used by other skills (`fs_read_bytes` in `skills/mod.rs`) goes
through the same resolver, so `image_*`, `binary_*`, `wave_*`,
`pcap_*`, `notebook_*`, `disasm_*`, etc. inherit the constraint
without each one re-implementing it.

> **Where**: `src/skills/filesystem.rs::resolve`,
> `src/skills/mod.rs::fs_read_bytes`.

## Constellation privacy model

The mesh is designed so a malicious or compromised peer can't extract
queries or data from us beyond what we explicitly cache + share.

- **Wire format**: only `hash_key(canonical(query))` (FNV-1a → hex)
  crosses the wire on a query consult. Raw query text never leaves the
  node. `src/constellation/bloom.rs::hash_key`.
- **Digests**: published every `[network].sync_secs` are
  `{node_id, constellation_id, peer_count, bloom: BloomFilter,
  peers: [URLs], delegation_enabled}`. **No row counts of memory,
  no cached payloads, no auth material.** Bloom filters are
  per-cache-hash, not per-row-body.
- **Consensus gate**: a result is only returned without a fresh local
  search when `>= [network].min_agreement` peers corroborate it,
  weighted by reputation, with single-peer influence capped. This
  prevents a malicious peer from injecting a poisoned answer alone.
  Default `min_agreement = 2`.
- **Reputation EMA**: each peer's hit rate is exponentially averaged;
  a peer that consistently disagrees with local truth decays toward
  0 and gets dropped from consults.
- **Storm guard**: the same query reaching us via multiple paths is
  re-fanned at most once per short window; duplicates fall back to a
  local-only answer. Prevents amplification attacks.
- **Self-peer dedup**: `Constellation::local_urls` tracks every
  address our own mDNS announces us on; `add_peer` consults the set
  so a peer can't gossip our address back to us. Prevents
  self-looping and self-counting.

> **Where**: `src/constellation/mod.rs` (top-level docs +
> `add_peer` + `consult` + `local_urls`),
> `src/constellation/bloom.rs`,
> `src/constellation/delegation.rs`.

## Browser sandbox

The browser session manager runs a single Chromium process and gives
each session its own isolated `BrowserContext` (separate cookies /
localStorage / cache). Sessions are **local-only** — the constellation
does NOT carry browser actions today. The risks and current controls:

- **Resource exhaustion**: cap on concurrent sessions
  (`max_concurrent`, default 8) + idle reaper that closes any session
  untouched for `idle_timeout_secs` (default 1800). A model that
  abandons a session can't pin a tab forever.
- **Per-session serial lock**: `Session.serial` is awaited before
  every action, so two tools running on the same session can't
  interleave CDP calls and leave the page in an inconsistent state.
- **Isolation**: each session is its own `BrowserContext` — sessions
  cannot read each other's cookies, even though they share one
  Chromium process. Calling `browser_close` disposes the context,
  which both closes the page and frees the per-context state.
- **No persistent profile**: ephemeral by design. A restart wipes
  every session. Logins and CAPTCHA-solved cookies do not survive.
- **JS surface**: `browser_eval` runs arbitrary JS in the page. This
  is only as dangerous as the URL the session has open — Chromium's
  same-origin policy is what keeps it from reading other origins.
  The model can already navigate anywhere via `browser_navigate`,
  so `browser_eval` doesn't expand the trust surface.

**Open follow-ups** (tracked in tasks 127–130):
- **SSRF protection on delegated browser work** (#130). When peers
  can delegate browser actions to us, navigation to RFC1918,
  loopback, link-local, IPv6 ULA, and `.local` (mDNS) hosts must be
  refused. Same restriction extends to the `fetch()` surface
  reachable through `browser_eval`. Local calls (the node's own
  MCP client) stay unrestricted; only the delegated path is gated.
- **Per-node capability gates** (#129). Each node will publish a
  capability set in its digest (`{browser: true, file_sharing:
  false, ...}`). Delegation requests for a capability the peer hasn't
  opted in to are refused before reaching the relevant skill. A new
  `constellation_capabilities` tool answers "who in the mesh can do
  X".
- **Poisoned-session detection** (#127) — auto-detect CAPTCHA /
  429 / 403 patterns, mark the session "suspect", route queries to a
  fallback (fresh context or another provider), require operator
  confirmation in the dashboard before reusing.

> **Where**: `src/skills/browser_session.rs` (manager + tools),
> `src/browser.rs` (underlying renderer).

## File store & memory data

- **File store** (`[store]`): writes go under `.lodestone-store/`,
  relative to the server's cwd. The path is configurable but is
  always treated as opaque blobs keyed by content hash — no
  user-supplied filenames are written.
- **Memory store** (`[memory]`): SQLite under
  `[memory].dir` (default `.lodestone-memory/`). All queries are
  parameterized; user-controlled strings never reach raw SQL. Memory
  contents stay node-local — the WS snapshot carries `COUNT(*)`
  only, never row bodies.
- **Constellation digest never carries memory data**: the bloom
  filter is built from cache keys (search/retrieval results — public
  web data), not from memory rows. Even when a peer's bloom matches
  a hash, the only thing it can pull back is a cached search-result
  payload, never a memo / solution / conversation row.

> **Where**: `src/skills/memory.rs`, `src/store/`,
> `src/main.rs::ws_snapshot`.

## Database & code-execution tools

`db_query` and `redis_command` take the connection URL **at call
time** — they never store credentials on disk and never read them
from config. The model is responsible for not echoing the URL it
passed in; the server does not log call arguments. Writes go through
the guard.

`shell_run` and `python_run` run subprocesses. They are off by
default (`[shell].enabled = false`, `[python].enabled = false`).
When enabled, `shell_run` can be configured with an allowlist
(`[shell].allowed`) so only specific commands run; the unrestricted
mode requires an explicit opt-in. Both go through the guard.

> **Where**: `src/skills/databases.rs`, `src/skills/shell.rs`,
> `src/skills/python.rs`.

## Dashboard ephemeral settings (`/api/settings/*`)

All four endpoints (`server`, `memory`, `constellation`, `tools`,
`browser`) share:

- Same Bearer-against-`[network].token` auth (constant-time).
- Sparse JSON patch — every field is optional; only the ones set are
  applied. Unknown fields are silently ignored to keep the surface
  forward-compatible.
- Clamped numeric inputs so a typo in the dashboard can't disable
  the consensus check or set a 4-billion-second timeout.
- Secrets are NEVER accepted: the patch types don't include
  `auth_token`, `network_token`, or any API key field. The dashboard
  has no UI to set them either.
- Tools `runtime_disabled` set is bounded to known tool names (the
  endpoint ignores unknown names rather than growing the set).
- Memory `enabled` toggle is a runtime gate, not a destructive
  action — the SQLite store stays open; turning the toggle on later
  restores access without restart or data loss.

> **Where**: `src/main.rs::api_routes`.

## Threat model: what we DO NOT defend against (today)

Explicit non-goals — surface them so an operator running in an
environment that needs more knows where the extra controls have to
come from:

- **Malicious operator on the host**. A privileged user on the host
  can read the SQLite file, the config, and the process memory. Use
  filesystem-level encryption / per-user accounts.
- **Multi-tenant on one process**. There is no per-tenant
  authentication on the MCP endpoint. The expected deployment is
  one server per principal.
- **A compromised constellation token compromising the mesh**. A
  leaked token lets the holder query our cache + submit
  delegated-retrieval requests (rate-limited). Rotate the token,
  ideally per-host, and don't paste it into chat.
- **Side channels via Chromium's renderer process**. The browser is
  isolated per-context, but Chromium-level zero-days (Rowhammer,
  Spectre) are out of scope.
- **DOS via the dashboard**. The WS push is open to anyone with the
  token; a misbehaving client can ask for snapshots repeatedly.
  Defended by `[network].token` and the natural cap on snapshot rate
  (`ws::PUSH_INTERVAL`, default 5s).

## Quick reference — control surfaces by file

| Surface                            | Code                                         |
| ---------------------------------- | -------------------------------------------- |
| MCP auth                           | `src/main.rs::ws_routes` / mcp service       |
| Constellation auth                 | `src/constellation/mod.rs::token_ok`         |
| Secret redaction                   | `src/skills/meta.rs` (`features` tool)       |
| Destructive guard                  | `src/skills/guard.rs`                        |
| Filesystem path resolver           | `src/skills/filesystem.rs::resolve`          |
| Constellation hash + bloom         | `src/constellation/bloom.rs`                 |
| Self-peer dedup                    | `src/constellation/mod.rs::local_urls`       |
| Delegation rate limiter            | `src/constellation/delegation.rs`            |
| Browser session isolation          | `src/skills/browser_session.rs`              |
| Settings endpoints                 | `src/main.rs::api_routes`                    |
| Constant-time bearer compare       | `src/util.rs::ct_eq`                         |

## Open security tasks

These show up in the project task tracker as well; listed here so the
auditor sees them in context:

- **#129** — Per-node capability gates. Each node opts in to which
  features it offers the mesh (browser, file-sharing, …). Delegation
  requests for a capability the peer hasn't opted in to are refused.
- **#130** — Delegated-browser SSRF protection. Refuse navigation
  (and `fetch()` via `browser_eval`) to RFC1918, loopback,
  link-local, IPv6 ULA, and `.local` hosts on delegated requests.
- **#127** — Poisoned-session detection + dashboard-confirmed reset
  for browser pools.
- **#128** — Constellation delegation for browser pool queries
  (queries delegate, sessions don't transport).
