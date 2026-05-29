# Constellation — peer-to-peer shared query knowledge

The constellation is an **opt-in** layer that lets lodestone instances consult each
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
  and latency any one query can incur.

## Avoiding request storms

A consult can fan out and (with `relay_hops > 0`) be forwarded a hop or two toward a
holder, and the galaxy links many constellations into a larger mesh — so the system
is designed so the *same* query is never screamed around repeatedly:

- **Hop ceiling.** Relay TTL is clamped to `relay_hops` (max **2**). A forwarded
  query can only travel a couple of hops before it stops, so depth is hard-bounded.
- **Path loop-guard.** Every relayed query carries a `seen` set of node ids; a node
  that finds itself already in `seen` returns nothing. Node ids are machine-derived
  and globally unique, so this breaks loops *within and across* constellations alike.
- **Cross-path dedup.** `seen` only covers one path, so the same key could still
  arrive via several paths. Each node therefore **relays a given key at most once per
  short window** — duplicates answer from local cache only and are *not* re-fanned.
  This collapses the multiplicative fan-out that would otherwise cause a storm.
- **Targeted fan-out.** Relays go only to **Bloom-matching** peers (likely holders),
  not to everyone, and are capped by `max_peers`.
- **One vote per peer.** However a top-level peer answered (directly or via relay),
  it counts as exactly one consensus vote — relaying can't fabricate corroboration.
- **The galaxy never proxies.** The broker only hands back endpoints; it never
  forwards a query, so it adds no amplification path. Cross-constellation consults are
  ordinary direct calls, subject to all of the above.
- **Cache short-circuit.** A node that already has the answer cached returns it
  immediately and relays nothing.

## Matching reworded queries

Peers match by the **hash** of a query key, so two nodes only share a result when
they compute the *same* key. To stop trivial wording differences from fragmenting
that, the key's text is **canonicalized** before hashing: lowercased, de-punctuated,
stop-words and excess whitespace removed — but **word order is preserved**, so
direction-sensitive phrasings stay distinct (`json to yaml` ≠ `yaml to json`). So
"How do I parse JSON in Rust?" and "parse json rust" already hit the same entry.

For genuinely different phrasings of the same need, enable `[search].fuzzy_match`.
Each search is then *also* keyed by an order-independent **concept signature** (a
stemmed, de-duplicated token set), and that concept hash is advertised in the digest
Bloom like any other key — so a peer that cached an equivalent query is found on an
exact-key miss, through the same consult + consensus path (no protocol change, still
hash-only on the wire). It's off by default because a bag-of-words signature is
order-insensitive and can collide on direction-sensitive queries. (A SimHash-based
*near*-duplicate match — catching one-token-different queries — is a deferred
extension; see [TODO.md](../TODO.md).)

## How it works

1. **Discovery & gossip.** Static `[network].peers` plus, when `[network].mdns`
   is on, LAN auto-discovery via mDNS (`_lodestone._tcp.local.`, advertising the
   node id in a TXT record so a node skips itself). On top of that, each digest
   **gossips** the peers a node knows, so the mesh grows from a seed; peers that
   fail repeatedly are pruned.
2. **Digests.** Every `sync_secs`, each node fetches peers' `GET /constellation/digest` —
   a Bloom filter of the query-key hashes they currently have cached, plus their
   known peers (for gossip) — which also builds the **graph** of who-knows-whom.
3. **Consult-then-fetch.** On a search, after a local cache miss, the node asks
   the peers whose Bloom filter *might* contain the key (`POST /constellation/query` with
   the hash). If consensus is reached (`>= min_agreement` corroborating peers), it
   returns that merged result labelled `constellation` and **skips re-scraping**. Otherwise
   it runs a normal local search, caches it, and updates peer reputations by how
   well their hits matched the local truth.
4. **Relay (a hop or two).** When a node can't reach a holder directly, it asks
   reachable intermediaries to forward the query along the graph for up to
   `relay_hops` hops (clamped to 2). Each `/constellation/query` carries a `ttl` and a
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

The **`constellation_status`** tool (skill) returns this node's id and every known peer's
reputation, reachability, miss count, and the graph edges it advertised. It
reports that the constellation is disabled when `[network].enabled` is false.

The result cache (`[cache]`) is the shared substrate: a node serves peers from the
same cache it fills with its own searches. Enabling the network therefore implies
an active cache even if `[cache].enabled` is false.

## Endpoints

Mounted only when `[network].enabled`. All require `Authorization: Bearer
<[network].token>` when that token is set (a trust domain separate from the public
`auth_token`); `/health` and `/mcp` are unaffected.

| Method | Path | Body | Response |
| --- | --- | --- | --- |
| `GET` | `/constellation/digest` | — | `{ node_id, generation, count, bloom: { m, k, bits }, peers: [...] }` |
| `POST` | `/constellation/query` | `{ "key": "<hash>", "ttl"?: n, "seen"?: [ids] }` | `{ "hits": [...] }` or `204` |
| `POST` | `/constellation/blob` | `{ "key": "<hash>" }` | raw bytes (`application/octet-stream`) or `204` |
| `POST` | `/constellation/blobinfo` | `{ "key": "<hash>" }` | `{ "hash": "<content-hash>", "size": n }` or `204` |

`ttl`/`seen` are optional (default 0 / empty) — a plain `{ "key": … }` works and
just disables relay for that request.

## File sharing (blobs)

When the on-disk file store (`[store]`) is enabled, the digest's Bloom filter also
advertises the **file-store entry hashes**, and peers can pull a cached file's raw
bytes via `POST /constellation/blob` (addressed by `hash_key(url)` — the raw URL never crosses
the wire). `read_pdf` and `store_fetch` resolve a URL as **local store → a constellation peer
that has it → the source** (caching the result), so a PDF/file one node fetched
(arXiv, IETF, …) is served from the mesh instead of every node re-hitting the
rate-limited source. The retrieval *text* cache is shared the same way (also behind
the Bloom), so parsed page/RFC/doc text one node produced isn't recomputed by every node.

### Anti-tampering

A peer could serve corrupted or malicious bytes, so blobs are **corroborated, then
verified** before they're trusted:

1. The consumer asks Bloom-matching peers for the blob's **content hash** only
   (`POST /constellation/blobinfo`, no bytes).
2. It trusts a content hash only when **`>= [network].min_agreement` distinct peers
   agree** on it (reputation breaks ties) — the same anti-poisoning gate as search
   results, so a lone or malicious peer can't dictate content. With the default
   `min_agreement = 2`, a single holder is *not* trusted (the consumer falls back to
   the source); lower it to `1` to favor availability over corroboration.
3. It fetches the bytes from an agreeing peer and checks they hash to the agreed
   value (`hash_bytes`) before accepting; on any mismatch it re-fetches from the
   authoritative source.

The content hash is a deterministic 128-bit FNV digest (same family as the key
hashes), so every honest node computes the same value for identical bytes.

### Seed accounting

Each node tracks, per blob hash, how many bytes it has **served** to peers vs.
**fetched** from them (`served_bytes / fetched_bytes` = a BitTorrent-style *seed
ratio*). Surfaced by the `constellation_seeds` tool and shown per file in `store_list`.

### Node identity

Each node has a stable id derived from the **OS machine id** (`machine-uid`; falls
back to hostname, else random) mixed with the bind port — so two instances on one
host stay distinct, yet each is stable across restarts. Peers record each other's id
from their digests; `constellation_status`/`constellation_peers` show it. Override with
`[network].node_id`.

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
reads `constellation: N peers` — served from A without B scraping. `list_providers` and the
logs show the activity.

On a real LAN, leave `mdns = true` and omit `peers`; nodes find each other
automatically.

## Galaxy — linking constellations

A **constellation** is a single mesh of instances that discover each other directly
(static peers + LAN mDNS). A **galaxy** is the next layer up: a small **broker** that
links *multiple constellations* across networks. Configure it under `[galaxy]`.

> **Galaxy connectivity is entirely optional.** It is off by default and is never a
> dependency: a constellation is fully functional on its own (and a single instance
> works with no constellation at all). The galaxy only *adds* cross-network discovery
> of other constellations — nothing breaks, and no behavior changes, without it.

It is deliberately **not a proxy** — the broker never relays digests, queries, or
blobs. It only keeps a directory of `{ constellation id → public endpoint(s) }`. A
constellation registers its publicly-reachable **ingress** URL(s); peers fetch the
directory and then talk to each other **directly** over the normal `/constellation/*`
endpoints (under each constellation's own token + consensus rules). So at least one
host must be publicly reachable — usually the broker, plus each constellation's
ingress node(s) (a forwarded/open port).

Two sides (independent):

- **The broker is a separate binary**, `lodestone-galaxy` — *not* part of the main
  `lodestone-mcp` server (the MCP server + its constellation are the main app). Run it
  on a publicly-reachable host; configure it by env:

  ```sh
  LODESTONE_GALAXY_BIND=0.0.0.0:8077 \
  LODESTONE_GALAXY_TOKEN=optional-shared-secret \
  LODESTONE_GALAXY_TTL_SECS=90 \
    lodestone-galaxy            # or: lodestone-galaxy 0.0.0.0:8077
  ```

  Endpoints: `POST /galaxy/register` (also `…/heartbeat`) and
  `GET /galaxy/directory?id=<self>`; both honor the token. Entries expire after the
  TTL without a heartbeat. It only stores/returns endpoints — never proxies traffic.
- **Participate** (`[galaxy].servers = [...]` in the main app, with the constellation
  enabled): every `heartbeat_secs`, register this constellation (its `id` + `ingress`
  URLs) with each broker and pull the directory, adding other constellations'
  endpoints as peers.

**Distribution.** A constellation may advertise **several `ingress` endpoints** — all
are added as peers, spreading inbound load across member nodes. Egress is distributed
too: every member node runs its own galaxy client and registers independently.

**Join order.** A node **joins its own constellation first** — the galaxy client
waits out a warm-up (`join_warmup_secs`, returning sooner once a local peer appears)
so local discovery settles before it asks a broker about *other* constellations.

> **Expose only the constellation, not the MCP server.** Set `[network].bind` to a
> separate port so `/constellation/*` listens apart from `/mcp`; forward *that* port
> as your `ingress` and keep the MCP endpoint private. (See the constellation config.)

A node can be a pure broker (set `serve = true`, leave the constellation off), a pure
participant, or both.

## Deferred

A Redis-backed *shared* cache (multiple nodes behind one store) — so peers read
from a common cache instead of (or in addition to) consulting each other. See
[TODO.md](../TODO.md). (Gossip, bounded relay, and reputation persistence are now
implemented.)
