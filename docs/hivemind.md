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

1. **Discovery.** Static `[network].peers` plus, when `[network].mdns` is on,
   LAN auto-discovery via mDNS (`_lodestone._tcp.local.`, advertising the node id
   in a TXT record so a node skips itself).
2. **Digests.** Every `sync_secs`, each node fetches peers' `GET /hive/digest` —
   a Bloom filter of the query-key hashes they currently have cached.
3. **Consult-then-fetch.** On a search, after a local cache miss, the node asks
   the peers whose Bloom filter *might* contain the key (`POST /hive/query` with
   the hash). If consensus is reached (`>= min_agreement` corroborating peers), it
   returns that merged result labelled `hive` and **skips re-scraping**. Otherwise
   it runs a normal local search, caches it, and updates peer reputations by how
   well their hits matched the local truth.

The result cache (`[cache]`) is the shared substrate: a node serves peers from the
same cache it fills with its own searches. Enabling the network therefore implies
an active cache even if `[cache].enabled` is false.

## Endpoints

Mounted only when `[network].enabled`. Both require `Authorization: Bearer
<[network].token>` when that token is set (a trust domain separate from the public
`auth_token`); `/health` and `/mcp` are unaffected.

| Method | Path | Body | Response |
| --- | --- | --- | --- |
| `GET` | `/hive/digest` | — | `{ node_id, generation, count, bloom: { m, k, bits } }` |
| `POST` | `/hive/query` | `{ "key": "<hash>" }` | `{ "hits": [...] }` or `204` |

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

## Deferred (not in v1)

Gossip peer-exchange, reputation persistence across restarts, and a Redis-backed
*shared* cache (multiple nodes behind one store). See [TODO.md](../TODO.md).
