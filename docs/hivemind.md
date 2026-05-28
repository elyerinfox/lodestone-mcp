# Hivemind — peer-to-peer shared query knowledge

The hivemind is an **opt-in** layer that lets lodestone instances consult each
other's caches before scraping the open web. A query already answered by a peer
can be served from the network, spreading load and softening per-IP rate limits.

It is **never a dependency**: with zero peers (or the feature off) every instance
works exactly as a standalone server. It is **off by default** (`[network].enabled
= false`).

## Guarantees

- **Privacy.** Only *hashes* of normalized query keys cross the wire — never raw
  query text. Peers advertise a Bloom filter of the hashes they have cached;
  a query is a single hash; a response is a cached result list or nothing.
- **No secrets.** Responses contain only cached *search results* (public web
  data). Tokens/keys are never shared.
- **Anti-poisoning.** Peer data is untrusted. A result is returned without a local
  search only when at least `min_agreement` peers corroborate it; each peer's
  contribution is capped (`max_results_per_peer`); and peers are weighted by a
  reputation score (EMA toward how well they agree with consensus / local truth,
  decayed toward neutral when unreachable). No single peer can carry a result.
- **Bounded.** `max_peers`, `request_timeout_ms`, and capped lists bound the work
  and latency a query can incur. A peer is never asked to relay — it answers only
  for its own cache (no query amplification).

## How it works

1. **Discovery & gossip.** Static `[network].peers` plus, when `[network].mdns`
   is on, LAN auto-discovery via mDNS (`_lodestone._tcp.local.`, advertising the
   node id in a TXT record so a node skips itself). On top of that, each digest
   **gossips** the peers a node knows, so the mesh grows from a seed; peers that
   fail repeatedly are pruned.
2. **Digests.** Every `sync_secs`, each node fetches peers' `GET /hive/digest` —
   a Bloom filter of the query-key hashes they currently have cached, plus their
   known peers (for gossip) — which also builds the **graph** of who-knows-whom.
3. **Consult-then-fetch.** On a search, after a local cache miss, the node asks
   the peers whose Bloom filter *might* contain the key (`POST /hive/query` with
   the hash). If consensus is reached (`>= min_agreement` corroborating peers), it
   returns that merged result labelled `hive` and **skips re-scraping**. Otherwise
   it runs a normal local search, caches it, and updates peer reputations by how
   well their hits matched the local truth.
4. **Relay (a hop or two).** When a node can't reach a holder directly, it asks
   reachable intermediaries to forward the query along the graph for up to
   `relay_hops` hops (clamped to 2). Each `/hive/query` carries a `ttl` and a
   `seen` node-id set: a peer serves from its own cache, else (while `ttl > 0` and
   not already visited) forwards to its bloom-matching peers one hop closer.
   Loops are broken by `seen`; fan-out is bounded by `max_peers` and the timeout.
   Crucially, each *top-level* peer is still exactly **one** consensus vote no
   matter how many sub-peers it relayed through — so relaying can't manufacture
   corroboration.
5. **Reputation persistence.** With `[network].state_file` set, peer reputations
   are written there after each sync and reloaded on startup, so earned trust
   survives restarts.

## Inspecting the mesh

The **`hive_status`** tool (skill) returns this node's id and every known peer's
reputation, reachability, miss count, and the graph edges it advertised. It
reports that the hivemind is disabled when `[network].enabled` is false.

The result cache (`[cache]`) is the shared substrate: a node serves peers from the
same cache it fills with its own searches. Enabling the network therefore implies
an active cache even if `[cache].enabled` is false.

## Endpoints

Mounted only when `[network].enabled`. Both require `Authorization: Bearer
<[network].token>` when that token is set (a trust domain separate from the public
`auth_token`); `/health` and `/mcp` are unaffected.

| Method | Path | Body | Response |
| --- | --- | --- | --- |
| `GET` | `/hive/digest` | — | `{ node_id, generation, count, bloom: { m, k, bits }, peers: [...] }` |
| `POST` | `/hive/query` | `{ "key": "<hash>", "ttl"?: n, "seen"?: [ids] }` | `{ "hits": [...] }` or `204` |

`ttl`/`seen` are optional (default 0 / empty) — a plain `{ "key": … }` works and
just disables relay for that request.

## Configuration

See [`config/06-network.toml`](../config/06-network.toml) for every option with
defaults and the matching `LODESTONE_NETWORK_*` env vars.

## Two-node test

```sh
# Node A (will fill its cache and serve B)
LODESTONE_BIND=127.0.0.1:8000 \
LODESTONE_NETWORK_ENABLED=1 LODESTONE_NETWORK_MDNS=0 \
cargo run

# Node B, pointed at A, with min_agreement=1 so a single peer is trusted in the
# demo (production keeps the default of 2+)
LODESTONE_BIND=127.0.0.1:8001 \
LODESTONE_NETWORK_ENABLED=1 LODESTONE_NETWORK_MDNS=0 \
LODESTONE_NETWORK_PEERS=http://127.0.0.1:8000 \
LODESTONE_NETWORK_NODE_ID=node-b \
cargo run
```

Run a `web_search` on **A** (fills A's cache). Within `sync_secs`, B pulls A's
digest. Run the *same* `web_search` on **B**: it returns a result whose `meta`
reads `hive: N peers` — served from A without B scraping. `list_providers` and the
logs show the activity.

On a real LAN, leave `mdns = true` and omit `peers`; nodes find each other
automatically.

## Deferred

A Redis-backed *shared* cache (multiple nodes behind one store) — so peers read
from a common cache instead of (or in addition to) consulting each other. See
[TODO.md](../TODO.md). (Gossip, bounded relay, and reputation persistence are now
implemented.)
