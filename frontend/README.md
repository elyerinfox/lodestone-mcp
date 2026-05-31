# lodestone-mcp dashboard

Nuxt 3 + Vue 3 + Tailwind dashboard that reads live snapshots from a
running `lodestone-mcp` server over WebSockets at `/ws/status`. Left-side
nav rail, responsive (collapses to a top bar on small screens), dark
theme to match the Grafana-style chart family.

## Quick start

```sh
cd frontend
npm install
npm run dev   # opens http://localhost:3000
```

The dev server defaults to a same-origin `ws(s)://<host>/ws/status`
WebSocket. When running the dashboard against a `lodestone-mcp` on
another host or port, set environment variables before `npm run dev`:

```sh
NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status npm run dev
# or, with a configured [network].token:
NUXT_PUBLIC_WS_URL=ws://localhost:8000/ws/status \
  NUXT_PUBLIC_WS_TOKEN=smoke-token \
  npm run dev
```

## Layout

| Path | Purpose |
| --- | --- |
| `app.vue` | Root wrapper, mounts `NuxtLayout` + `NuxtPage`. |
| `layouts/default.vue` | Two-column shell: left nav rail (md+) / top bar (sm), main column with the routed page. Owns the WebSocket connection via `useDashboardFeed()` and `provide()`s it down to child pages. |
| `pages/index.vue` | Overview — headline stats from all three subsystems. |
| `pages/memory.vue` | Memory store counts (memos, solutions, conversations, …). |
| `pages/constellation.vue` | Node identity + peer table + delegation knobs + seed accounting. |
| `composables/useDashboardFeed.ts` | Owns the WebSocket lifecycle (auto-reconnect, snapshot parsing). |
| `components/StatCard.vue` | Big-number tile. |
| `components/SectionHeading.vue` | Uppercase section header. |
| `types/ws.ts` | TypeScript mirror of `src/ws.rs`'s envelope. |

## Backend contract

See [`docs/frontend.md`](../docs/frontend.md) for the full wire-format
spec. Short version: the server pushes a single
`{ "type": "snapshot", "data": { server, memory, constellation } }` JSON
message on connect, then refreshes every 5 s for as long as the socket
is open. Auth via `?token=<network-token>` for parity with the
constellation endpoints.

## What's not here yet (deferred)

- Memory-row browser (the feed carries counts only — viewing individual
  memos / solutions needs a separate authenticated REST endpoint).
- Constellation graph visualization (currently a peer table; a force-
  directed graph over `peers[*].known` would show the mesh shape).
- Action verbs — delegated retrieve trigger, solution forget, conversation
  prune. The feed is push-only for v1; these need a second auth-checked
  channel.
- Production build embedded in the lodestone-mcp binary via
  `include_dir!`. The dev workflow runs the Nuxt dev server next to the
  backend.
