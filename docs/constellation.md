# Constellation — peer-to-peer shared query knowledge

The constellation is an **opt-in** layer that lets lodestone instances consult each
other's caches before scraping the open web. A query already answered by a peer
can be served from the network, spreading load and softening per-IP rate limits.

It is **never a dependency**: with zero peers (or the feature off) every instance
works exactly as a standalone server. It is **off by default** (`[network].enabled
= false`).

## Terminology

These terms are used throughout this page (and in the config/tools); definitions
come first so the rest reads unambiguously.

**Structure**

- **Instance** / **node** — one running `lodestone-mcp` server. The two words are
  used interchangeably here ("node" when talking about the graph, "instance" when
  talking about the process).
- **Peer** — another instance this node knows about and can consult.
- **Constellation** — a single mesh of instances that discover each other **directly**
  (static `[network].peers` + LAN mDNS) and share caches. The base layer; this whole
  page is about it unless the galaxy is named.
- **Galaxy** — an *optional* layer above constellations that links **multiple
  constellations** across networks. See [Galaxy](#galaxy--linking-constellations).
- **Broker** — the separate `lodestone-galaxy` binary at the center of a galaxy: a
  directory of `{ constellation → public endpoint(s) }`. It **never proxies** traffic;
  it only hands back endpoints so constellations talk directly.

**Identity**

- **`node_id`** — a stable id for *one instance* (OS machine id + bind port; override
  `[network].node_id`). Unique per process, stable across restarts.
- **constellation id** — a *shared* id for all members of one constellation
  (`[network].id`, distinct from `node_id`). Nodes that reach each other **converge to
  the larger mesh's id** (alphabetically smaller id as the tiebreaker on equal sizes),
  so a mesh registers in the galaxy as one entry, not one per node, and a small mesh
  meeting a big one is absorbed by the big one.
- **`ingress`** — a constellation's publicly-reachable URL(s), registered with a broker
  so other constellations can connect inbound.

**Mechanism**

- **Digest** — what a node publishes every `sync_secs` at `GET /constellation/digest`:
  a Bloom filter of the key hashes it has cached, plus its known peers (for gossip).
- **Bloom filter** — a compact probabilistic set: lets a peer ask "do you *maybe* have
  this hash?" without listing contents. Hashes only — never raw query text.
- **Consult / consensus** — on a local cache miss, a node *consults* Bloom-matching
  peers for a key and trusts the answer only when `>= min_agreement` peers
  **corroborate** it (reputation-weighted) — the anti-poisoning gate.
- **Relay** — forwarding a consult a hop or two (≤ `relay_hops`, max 2) toward a node
  that holds the key, for peers not reachable directly.
- **Gossip** — including known-peer lists in the digest so the mesh grows from a seed.
- **Reputation** — a per-peer score (EMA) of how well a peer's answers match consensus
  / local truth; weights its votes and decays toward neutral when unreachable.

**Data shared**

- **Blob** — a cached *file's raw bytes* (from the `[store]` file store, e.g. a fetched
  PDF), shared over the mesh addressed by content hash.
- **Seed ratio** — per-blob `served_bytes / fetched_bytes` (BitTorrent-style), surfaced
  by `constellation_seeds`.

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

## Request flows

The constellation isn't a black box — every academic / scientific
retrieval (golden rule 13) takes one of the paths below. The end-to-end
mechanism is `retrieval_get` / `retrieval_put` (defined in
[`src/main.rs`](../src/main.rs)) chained through the constellation's
Bloom-digest sync; this section spells out each branch.

### Path A — local cache hit (warm path, no network)

The fastest case. We've fetched this document before and it's still
within its TTL. Zero network calls.

```mermaid
sequenceDiagram
    autonumber
    participant S as Skill
    participant L as Local cache
    S->>L: retrieval_get(key)
    L-->>S: cached bytes
    Note over S,L: done — zero network calls
```

This is the path that fires when the same model asks the same query
twice in a session, or when a skill that emits aliases (e.g.
`retrieval_put_indexed` with primary id + URL + content hash) is queried
by an alternate identifier the caller chose.

### Path B — peer cache hit (mesh-served, upstream untouched)

We don't have it locally, but a peer's Bloom digest claims they do.
This is the "helping the greater good" path that GR #13 is built on.

```mermaid
sequenceDiagram
    autonumber
    participant S as Skill
    participant L as Local cache
    participant P as Peer X
    S->>L: retrieval_get(key)
    L-->>S: miss
    Note over S: hash the key, check peer Blooms
    S->>P: POST constellation query with hashed key
    P-->>S: cached bytes
    S->>L: cache.put(key, body)
    Note over S: return bytes — upstream NEVER touched
```

Notes:
- The hash on the wire is `h`, not the raw key string. Raw queries
  do not traverse the mesh (see [Privacy](#guarantees)).
- If multiple peers' Blooms match, we ask up to `[network].max_peers`
  in parallel and apply the consensus floor `[network].min_agreement`.
  Below the floor we treat it as a miss and continue to Path C.
- Bloom filters have a small false-positive rate. A peer returning
  "not in my cache" after a Bloom match costs one wasted RTT; we
  continue to the next candidate.

### Path C — mesh miss, upstream OK (cold path, we become the seed)

Brand-new document; nobody on the mesh has it; the upstream is
reachable. We fetch from the upstream and then advertise the result
so peers don't have to.

```mermaid
sequenceDiagram
    autonumber
    participant S as Skill
    participant L as Local cache
    participant P as Peers
    participant U as Upstream
    S->>L: retrieval_get(key)
    L-->>S: miss
    S->>P: check advertised Blooms
    P-->>S: no match
    S->>U: HTTP fetch
    U-->>S: document bytes
    S->>L: retrieval_put(key, body)
    Note over S,L: on the next constellation sync the key hash<br/>is added to OUR advertised Bloom filter
```

After step 7, any peer that subsequently calls `retrieval_get` for the
same artifact will hit us via Path B and avoid the upstream entirely.
That's the "ratchet" that gives the constellation its value — every
cold-path fetch raises the warm-path coverage of the whole mesh.

For tools whose aliases are known at fetch time (arxiv id ↔ abs URL
↔ pdf URL; DOI ↔ raw URL; UniProt accession ↔ entry name), the skill
calls `retrieval_put_indexed(Identifiers, body)` instead so the
artifact is discoverable under every public name. That's what closes
the alignment gap for long-tail content — a peer asking by URL still
finds an entry we cached by DOI, and vice versa (see Path E).

### Path D — mesh miss, upstream rate-limited (the honest gap)

Brand-new document, nobody on the mesh has it, AND the upstream is
returning 429 / CAPTCHA / quota exceeded.

```mermaid
sequenceDiagram
    autonumber
    participant S as Skill
    participant L as Local cache
    participant P as Peers
    participant U as Upstream
    S->>L: retrieval_get(key)
    L-->>S: miss
    S->>P: check advertised Blooms
    P-->>S: no match
    S->>U: HTTP fetch
    U-->>S: 429 or 403 or CAPTCHA
    Note over S: skill returns error to the model<br/>NO retrieval_put — nothing to share<br/>NO peer-relay (today)
```

**The constellation does not "ask a peer to fetch on our behalf"**.
Today it is a passive cache of what peers have already fetched
themselves — not a fetch-relay across other nodes' IPs / quotas /
credentials. The mitigations at this point are skill-level, not
constellation-level:

- **Search**: `[search].strategy = fallback` walks an ordered provider
  list (DDG → Mojeek → SearXNG → keyed engines), so a CAPTCHA wall on
  one engine doesn't kill the call.
- **`wayback_fetch`**: explicit Internet Archive fallback for an
  individual URL that 404s on the live web.
- **`render=true`**: routes the fetch through headless Chrome; helps
  with some scrape-blocked sites but doesn't bypass account-bound
  rate limits.
- **Per-provider proxy** (`[providers].<id>.proxy`): outbound proxy
  that doesn't share the blocked egress IP.

An "opt-in fetch relay" — where a node says "I'm rate-limited on this
key, will any peer fetch and `retrieval_put` it for me?" — is a design
point we have **not** built. The trade-offs (the relay burns its own
quota; trust that the relay didn't tamper; privacy / what the relay
learns; abuse / one bad node spamming the mesh; spec creep from
passive cache to active forwarding) are real and are tracked in
[TODO.md](../TODO.md).

### Path E — alias hit via multi-identifier index

The artifact exists on a peer under one identifier (say, the raw URL
the peer fetched from), and we ask by a different identifier (the DOI
the skill prefers). The constellation finds it because the peer used
`retrieval_put_indexed` with both aliases.

```mermaid
sequenceDiagram
    autonumber
    participant S as Skill
    participant L as Local cache
    participant P as Peer X
    Note over P: previously cached with multiple alias hashes<br/>all advertised on the same Bloom
    S->>L: retrieval_get(doi_key)
    L-->>S: miss
    Note over S: h_doi matches peer X Bloom via the alias
    S->>P: POST constellation query with h_doi
    P-->>S: bytes looked up via alias-hash index
    S->>L: cache.put
```

This is the path that closes the alignment gap on long-tail content.
Without aliases, two nodes asking for the same paper by different
identifiers would each fetch from the upstream independently.

### Path F — relayed peer-of-peer hop

We don't have it; our direct peers' Blooms don't match; but a peer
two hops away does. Relay carries the query along the graph for up to
`[network].relay_hops` hops (clamped to 2 in the current implementation).

```mermaid
sequenceDiagram
    autonumber
    participant Us as Us
    participant A as Peer A
    participant B as Peer B
    Us->>A: POST query — ttl=2, seen contains Us
    Note over A: local miss<br/>Bloom — peer B claims a match<br/>seen now lists Us and A, ttl becomes 1
    A->>B: POST query — ttl=1, seen contains Us and A
    Note over B: local HIT
    B-->>A: bytes
    A-->>Us: bytes
    Us->>Us: cache.put
    Note over Us: A counts as ONE consensus vote no matter<br/>how many sub-peers it relayed through —<br/>relay cannot manufacture corroboration
```

Constraints that keep relay honest:

- `ttl` decrements on every hop; ≤ 0 ⇒ no further forwarding.
- `seen` set carries every node id that touched the request; a peer
  that's already in `seen` won't re-forward (loop break).
- Each **top-level** peer is exactly one consensus vote no matter how
  many sub-peers it relayed through — relaying cannot manufacture
  corroboration to meet `min_agreement`.
- Fan-out is bounded by `max_peers` and the per-request timeout
  (`request_timeout_ms`).

### Path G — opted-out / constellation disabled

When `[network].enabled = false` (the default — joining a constellation
is a privacy decision), only Path A and a direct version of Path C
exist; the local cache still serves the same node's repeated queries,
but nothing crosses the mesh.

```mermaid
sequenceDiagram
    autonumber
    participant S as Skill
    participant L as Local cache
    participant U as Upstream
    Note over S: network.enabled = false (default)
    S->>L: retrieval_get(key)
    alt cache hit
        L-->>S: cached bytes
    else cache miss
        L-->>S: miss
        S->>U: HTTP fetch
        U-->>S: bytes
        S->>L: retrieval_put — local only, no advertise
    end
```

This is the safe-default mode. Operators who have not deliberately
joined a constellation (via static `peers` or mDNS) operate here.

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
| `GET` | `/constellation/digest` | — | `{ node_id, generation, count, bloom: { m, k, bits }, peers: [...], capabilities: { query, retrieval, blob, browser } }` |
| `POST` | `/constellation/query` | `{ "key": "<hash>", "ttl"?: n, "seen"?: [ids] }` | `{ "hits": [...] }` or `204` |
| `POST` | `/constellation/blob` | `{ "key": "<hash>" }` | raw bytes (`application/octet-stream`) or `204` |
| `POST` | `/constellation/blobinfo` | `{ "key": "<hash>" }` | `{ "hash": "<content-hash>", "size": n }` or `204` |
| `POST` | `/constellation/retrieve` | `{ "url", "max_bytes", "source"? }` | raw bytes or JSON reject; gated by `[network.capabilities].retrieval` (mirrors the legacy `delegation_enabled`). |
| `POST` | `/constellation/browser_persona` | `{ "persona_name", "url" }` | `{ "url", "title", "tree" }` or JSON reject; gated by `[network.capabilities].browser`. Per-peer pool isolation via `X-Lodestone-Peer-Id` (sessions never transport). SSRF guard refuses local-network URLs. |

`ttl`/`seen` are optional (default 0 / empty) — a plain `{ "key": … }` works and
just disables relay for that request.

## Capabilities

Each node publishes a per-feature opt-in set on its digest. Peers read
the set on the next sync tick and can pick based on it when looking
for a delegate. The local model's `constellation_capabilities` tool
turns the set into a "who in the mesh can do X?" lookup.

| Capability | Default | Gates |
| --- | --- | --- |
| `query` | ON | `/constellation/query` cache consults |
| `retrieval` | OFF | `/constellation/retrieve` URL-fetching jobs (alias of legacy `delegation_enabled`) |
| `blob` | ON | `/constellation/blob` + `/blobinfo` file-store bytes |
| `browser` | OFF | `/constellation/browser_persona` peer-hosted browser sessions |

Setting `[network.capabilities].browser = true` means peers can
ask us to drive named browser sessions on their behalf via
`/constellation/browser_persona`. The session manager tracks those in
a separate "guest sessions" registry from the model's own personas,
keyed by `(peer_id, persona_name)` so two peers never share cookies
on the same logical name. Each request goes through the SSRF guard
(refuses RFC1918 / loopback / link-local / .local etc.) and
`browser_eval` is rejected on guest sessions outright. When the peer
drops out of our peer table, its guest sessions are evicted in one
sweep. See [`docs/skills/browser_session.md`](skills/browser_session.md)
for the operator-facing detail.

The constellation settings drawer on the dashboard can flip
capabilities at runtime — no restart needed.

## File sharing (blobs)

When the on-disk file store (`[store]`) is enabled, the digest's Bloom filter also
advertises the **file-store entry hashes**, and peers can pull a cached file's raw
bytes via `POST /constellation/blob` (addressed by `hash_key(url)` — the raw URL never crosses
the wire). `read_pdf` and `store_fetch` resolve a URL as **local store → a constellation peer
that has it → the source** (caching the result), so a PDF/file one node fetched
(arXiv, IETF, …) is served from the mesh instead of every node re-hitting the
rate-limited source. The retrieval *text* cache is shared the same way (also behind
the Bloom), so parsed page/RFC/doc text one node produced isn't recomputed by every node.

### Multi-identifier retrieval entries

The retrieval cache used to key each entry by a single canonical string (e.g.
`wayback|max|ts|<url>`). The mesh could only help a consumer that asked by the
*exact* same canonical key, which broke for long-tail rate-limited content
because two peers cached under different per-skill formats would never see
each other.

The cache is now a **multi-index** keyed by an [`Identifiers`](../src/constellation/identifiers.rs)
set:

- the **primary canonical key** (the per-skill cache string, hashed);
- **URL aliases** — every URL that resolves to the body (raw URL, resolved
  snapshot URL, redirected URL, …);
- **source-specific identifiers** — `("arxiv", "1706.03762v5")`,
  `("wayback_ts", "20240315120000")`, `("doi", "10.48550/arXiv.1706.03762")`,
  …, namespaced by [`Source`](../src/constellation/identifiers.rs) so different
  upstreams can't collide on identical `(label, value)` pairs;
- the **content hash** of the body (computed at put-time).

Each entry advertises **every** identifier hash in the digest Bloom (capped at
8 per entry to keep the digest small), so a peer that asks by any one of them
gets a hit. A consumer asking `https://arxiv.org/abs/1706.03762v5` finds an
entry a peer cached under the source-id `("arxiv", "1706.03762v5")` and vice
versa.

### Per-source consensus policy

Sources fall into two safety classes. `consult_blob_hash_sourced` accepts a
[`Source`](../src/constellation/identifiers.rs) hint so the right policy
applies per call:

| Source | TTL override | `min_agreement` floor | Why |
| --- | --- | --- | --- |
| `Wayback` | 7 days | **1** | `(url, timestamp)` is content-addressable; consumer-side bytes-hash check is the primary safety |
| `Arxiv` | 7 days | **1** | `{id}+v` is immutable per version; consumer can verify |
| `Github` (releases) | 7 days | **1** | Tag-addressable, immutable |
| `Overpass` | 1 day | `max(cfg, 2)` | World data changes slowly; consensus matters because bytes can't be verified from the query |
| `SearchEngine` | 1 hour | `max(cfg, 2)` | Volatile; consensus matters |
| `Other` | global default | global default | Fallback for entries that don't classify their source |

For **content-addressable sources** the `min_agreement` floor drops to **1**
*regardless* of `[network].min_agreement`. The safety doesn't come from peer
consensus — it comes from the consumer's content-hash check (step 3 of
"Anti-tampering" below). Requiring multiple peers to corroborate a hash a
single peer derived from the same identifier the consumer was looking up by
adds latency without adding safety. This is the change that makes long-tail
rate-limited content (a specific arXiv paper, a specific Wayback snapshot)
actually serveable across the mesh — usually only one peer in the constellation
has any given long-tail entry.

For **volatile / non-content-addressable sources** the existing multi-peer
corroboration applies, and a user that hardens to
`[network].min_agreement = 3` is never silently relaxed.

The same per-source floor applies to the **search-result consensus path**
(`consult` → `consensus`, separate from the blob-hash path). Search results
are inherently `Source::SearchEngine` and use `max(cfg.min_agreement, 2)`:
even a user that relaxes `cfg.min_agreement = 1` doesn't accept lone-peer
search results, because there's no consumer-side verification for search
hits (you can't recompute a search ranking the way you can recompute a
content hash), so a single (potentially malicious) peer could otherwise
inject results.

### End-to-end delegation flow

```mermaid
sequenceDiagram
  autonumber
  participant App as App / Skill
  participant L as Lodestone<br/>(this node)
  participant Cache as Local cache<br/>(store + IndexedRetrieval)
  participant Peer as Peer node<br/>(constellation)
  participant Up as Upstream<br/>(rate-limited)

  App->>L: fetch_bytes_shared(url)
  L->>Cache: lookup_by_url
  alt local hit
    Cache-->>L: bytes
    L-->>App: bytes
  else local miss
    L->>Peer: consult_blob(url hash)<br/>+ blobinfo corroboration
    alt peer-cached + corroborated
      Peer-->>L: bytes
      L->>Cache: put bytes
      L-->>App: bytes
    else peer cache miss
      L->>Up: direct GET url
      alt upstream ok
        Up-->>L: bytes
        L->>Cache: put bytes
        L-->>App: bytes
      else upstream 429 / blocked
        Note over L,Peer: walk peers advertising<br/>delegation_enabled = true
        L->>Peer: POST /constellation/retrieve<br/>X-Lodestone-Peer-Id + url
        Peer->>Peer: try_acquire — rate limit check
        alt limiter rejects
          Peer-->>L: 429 / 413 + RetrieveReject
          L-->>App: upstream error
        else accepted
          Peer->>Up: fetch on behalf
          Up-->>Peer: bytes
          Peer->>Peer: cache locally<br/>(now mesh-visible)
          Peer-->>L: bytes
          L->>Cache: put bytes
          L-->>App: bytes
        end
      end
    end
  end
```

Five doors checked in order: **local store → peer cache → direct upstream →
peer-delegated fetch → error**. Steps 2 and 4 require `[network].enabled`;
step 4 additionally needs at least one peer advertising
`delegation_enabled = true`. With none of those configured this collapses to
plain HTTP.

### Cross-constellation transfer

```mermaid
flowchart LR
  subgraph C1["constellation X"]
    A[Node A1]
    A2[Node A2]
  end
  subgraph C2["constellation Y"]
    B[Node B1<br/>delegation_enabled]
    B2[Node B2]
  end
  Broker["galaxy broker<br/>directory only"]

  A -.register endpoints.-> Broker
  B -.register endpoints.-> Broker
  A -.pull directory.-> Broker
  B -.pull directory.-> Broker

  Broker -. "introduces<br/>(no proxying)" .-> A
  Broker -. "introduces<br/>(no proxying)" .-> B

  A ==>|"add_peer<br/>(direct)"| B
  A ==>|"POST /constellation/retrieve<br/>(direct, no broker)"| B

  style Broker fill:#fff5e6,stroke:#c08000
  style B fill:#e6ffe6
```

The broker is a **directory**, not a proxy — it learns each constellation's
ingress endpoints and tells the others. Once introduced, constellations
talk **direct**: a node in X adds nodes in Y as peers via the normal
`add_peer`, fetches their digest, sees `delegation_enabled = true`, and
POSTs `/constellation/retrieve` directly. The bytes never touch the broker.

### Retrieval delegation (opt-in)

When local cache + peer-cache both miss and the direct upstream fetch fails
(rate-limited, geo-blocked, captive-portalled), a peer that has opted into
delegation can perform the fetch on the consumer's behalf. The asking node
POSTs `/constellation/retrieve { url, max_bytes, source }`; the serving
node fetches from upstream, caches the body locally (so it serves the mesh
via the Bloom-gated `consult_blob_hash` path going forward), and returns
the bytes. The rate-limited upstream is hit **once for the mesh**, not
once per node.

**Lookup order for `fetch_bytes_shared` is therefore:**

1. **Local file store** — already-downloaded bytes.
2. **Peer cache** (via `consult_blob`) — a peer that has it served the
   bytes; consensus / verification applies per source.
3. **Direct upstream fetch** — plain HTTP. On any error (typically 429),
   fall through.
4. **Peer-delegated fetch** — ask a peer that advertised
   `delegation_enabled = true` to fetch it for us.

Steps 2 and 4 require `[network].enabled` and at least one peer; step 4
additionally requires at least one peer that advertised
`delegation_enabled = true` on its most recent digest. The path collapses
gracefully — with none of those configured, this is just a plain HTTP
download.

**Cross-constellation delegation** works automatically through the galaxy
broker's directory mechanism: when a foreign constellation's endpoints are
added as peers via `galaxy::client::sync_once`, their digests start being
fetched, the `delegation_enabled` flag becomes visible, and
`delegated_fetch` walks all peers regardless of constellation origin. The
broker itself is **not a proxy** — bytes flow constellation-to-constellation
direct, the broker only made the introduction.

**Guardrails (server side).** When a serving node opts in
(`[network].delegation_enabled = true`), three sliding-hour-window counters
protect it from being used as someone else's exit:

| Knob | Default | What it caps |
| --- | --- | --- |
| `delegation_max_jobs_per_peer_per_hour` | 30 | Jobs any one peer can request per hour. A misbehaving peer fills its own quota first. |
| `delegation_max_bytes_per_job` | 8 MiB | Body size of a single delegated fetch. |
| `delegation_total_bytes_per_hour` | 256 MiB | Aggregate bytes served via delegation per hour. Protects local egress. |
| `delegation_max_cache_bytes` | 64 MiB | Summed body size of all retrieval-cache entries (delegated or not). Eviction is oldest-by-expiry. 0 = unlimited. |

Rejected requests return HTTP 429 (peer-jobs / global-bytes) or 413 (per-job
size) with a JSON body carrying a machine-readable `reason` and a
`Retry-After` hint in seconds, so requesters back off intelligently rather
than re-bombard the peer.

**Privacy.** The requested URL **does** cross the wire here — the serving
node has to know what to fetch — so delegation is strictly opt-in for the
serving node, never on by default. The requester has no privacy obligation
since it knows the URL it asked for; the constellation `token` gates who
can ask in the first place.

### Anti-tampering

A peer could serve corrupted or malicious bytes, so blobs are **corroborated, then
verified** before they're trusted:

1. The consumer asks Bloom-matching peers for the blob's **content hash** only
   (`POST /constellation/blobinfo`, no bytes).
2. It trusts a content hash only when **`>= min_agreement` distinct peers
   agree** on it (reputation breaks ties) — the same anti-poisoning gate as search
   results, so a lone or malicious peer can't dictate content. The effective
   `min_agreement` is computed per call from the per-source policy table
   above: 1 for content-addressable upstreams, `max(cfg, 2)` for everything
   else.
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

## Auto-healing on network change

A constellation **survives a node moving between networks**. The path is
fully unattended — nothing needs restarting, no configuration touches —
because every part of the design assumes the mesh is unreliable.

What actually happens when a laptop in mesh **alpha** at home moves to a LAN
with peers already in mesh **beta**:

1. **Old peers age out.** Each `sync_secs` cycle (default 30s) the node tries
   to fetch every known peer's `/constellation/digest`. The Network 1 IPs
   don't route from Network 2, so the fetches fail. After `MAX_PEER_MISSES`
   (5 consecutive failures = ~2.5 min on defaults) each old peer is pruned.
   Their `Peer` record is dropped, in-flight `consult_blob_hash` /
   `delegated_fetch` against them time out per `[network].request_timeout_ms`
   and the chain falls through to the next door (direct upstream).
2. **New peers appear via two channels.** **mDNS** (if
   `[network].mdns = true`) announces under `_lodestone._tcp.local.` on the
   new LAN within seconds; other lodestone instances on the same broadcast
   domain auto-discover. **Gossip** then propagates: each peer's digest
   carries a sample of its own known peers, so once the laptop talks to
   *one* node on Network 2 it learns about the rest. If neither mDNS nor a
   `[galaxy]` broker nor `[network].peers` matches anything reachable, the
   node runs as a lone constellation until discovery — its caches still
   work locally with zero peers.
3. **The constellation_id merges to the *larger* mesh** (tiebreak: smaller
   alphabetical id). Each digest carries the advertiser's
   `constellation_id` AND its full `peer_count`. When the moved node sees
   a digest from a peer in a *different* constellation, `maybe_adopt_id`
   compares the two meshes:
   - **Different sizes** → the **larger** mesh's id wins. A 50-node
     `"alpha"` mesh meeting a 2-node `"beta"` mesh adopts `"alpha"` —
     even if `"beta"` < `"alpha"` alphabetically — because the more-
     defined mesh is the more useful name to converge on. The smaller
     mesh is the one that should adopt; otherwise mesh size carries no
     signal and merges would oscillate.
   - **Equal sizes** → the alphabetically-smaller id is the tiebreaker, so
     both ends compute the same answer with no extra state.
   - **Same id already** → no-op (already converged).

   **The change propagates by gossip.** When node X adopts a new
   constellation_id from peer Y, X's *next* `digest()` carries the new id.
   X's other peers — which may not yet have talked to Y — see X's new id
   on their next sync cycle and run the same comparison: most of the time
   they're now smaller than Y's mesh (because X's mesh just merged in) and
   they adopt too. The change spreads through the connected mesh in
   `O(sync_secs × diameter)` — for a sparse 30-node mesh with the default
   30s sync, that's a few minutes for full convergence. Nothing ever
   needs to be told twice; each node makes the same comparison
   independently and arrives at the same answer.
4. **The cache is preserved — the moving node is a *bridge*.** Every entry
   in the laptop's `IndexedRetrievalCache` and file store survives the
   network switch (same process, in-memory). The new digest re-advertises
   every identifier hash. So an arXiv paper the laptop cached at home is
   served to office peers via the existing `consult_blob_hash` flow with
   **no re-fetch from arXiv**. A roaming laptop carries warm cache between
   networks, and the upstream is hit *once for both meshes combined*.
5. **The galaxy broker reregisters lazily.** `galaxy::client` heartbeats
   every `heartbeat_secs` with current `ingress` endpoints. After the move
   the heartbeat sends the new Network 2 IPs; the broker's TTL expires the
   old Network 1 registration. Other constellations pulling the directory
   get the new endpoints. Until the heartbeat fires (typically minutes),
   peers pulled via galaxy still see the laptop's old endpoints — same
   prune logic catches that.

**Practical edges**:

- **Rate-limit counters survive the move** (same process). A peer-id that
  hit its delegation quota on Network 1 stays at quota when it reappears
  on Network 2.
- **Reputations** stay in memory across the move; `state_file` only
  persists across *restarts*.
- **No mDNS, no galaxy, no static peers configured on the new network** →
  laptop runs as a single-node constellation until something is reachable.
  Its caches keep serving locally to any skill on the same node, which is
  often enough.
- **The home mesh forgets the laptop** about 2-3 minutes after it
  disappears (one full `sync_secs` × `MAX_PEER_MISSES`).

```mermaid
sequenceDiagram
  participant L as Laptop
  participant H as Home peers (A1, A2)<br/>constellation "alpha"
  participant O as Office peers (B1, B2)<br/>constellation "beta"
  participant Gx as Galaxy broker (optional)

  Note over L,H: at home — converged on "alpha"
  L<<->>H: digests every 30s, cache shared

  Note over L: ===== moves networks =====
  L--xH: digest fetches fail (5×)
  Note over L,H: H prunes L from its table<br/>L prunes H from its table

  Note over L,O: mDNS announce on new LAN
  L<<->>O: digest exchange
  Note over L,O: maybe_adopt_id picks the LARGER mesh<br/>(alphabetical id is only the tiebreaker)<br/>then propagates to all peers via the next digest
  L-->>O: serves home-cached arxiv / wayback bytes<br/>(no upstream re-fetch)

  opt galaxy broker configured
    L->>Gx: heartbeat with new Network 2 ingress
    Gx-->>L: stale Network 1 reg ages out
  end
```

Short version: the constellation auto-heals, the moving node carries warm
cache between networks, and meshes converge by id when they meet — so two
co-located meshes a bridging node touches merge cleanly into one.

## Configuration

See [`config/06-network.toml`](../config/06-network.toml) for every option with
defaults and the matching `LODESTONE_NETWORK_*` env vars.

## Two-node test

```sh
# Node A (will fill its cache and serve B)
LODESTONE_BIND=127.0.0.1:8000 \
LODESTONE_NETWORK_ENABLED=1 LODESTONE_NETWORK_MDNS=0 \
cargo run --bin lodestone-mcp

# Node B, pointed at A, with min_agreement=1 so a single peer is trusted in the
# demo (production keeps the default of 2+)
LODESTONE_BIND=127.0.0.1:8001 \
LODESTONE_NETWORK_ENABLED=1 LODESTONE_NETWORK_MDNS=0 \
LODESTONE_NETWORK_PEERS=http://127.0.0.1:8000 \
LODESTONE_NETWORK_NODE_ID=node-b \
cargo run --bin lodestone-mcp
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

**One id per constellation.** Member nodes share a single **constellation id**
(`[network].id`; distinct from each node's `node_id`). It's random if unset, and
nodes that reach each other **converge to the larger mesh's id** — the smaller
mesh adopts the larger one's id, with the alphabetically-smaller id as the
tiebreaker when sizes are equal. So a multi-node constellation registers as
*one* entry in the galaxy (not one per node), and two meshes that find each
other on a network **merge** into a single constellation that keeps the
more-defined name. The galaxy client registers under this id unless
`[galaxy].id` overrides it.

**Bidirectional ("reach out" *and* "allow in").** Participation is two-way and
symmetric through the directory: **registering** your `ingress` makes you discoverable
so other constellations connect *to* you (traffic in), while **pulling** the directory
adds their endpoints as peers so you connect *to* them (reach out). Both happen each
cycle. A node with no public `ingress` can still pull-and-consult (outbound-only);
a node with `ingress` is also reachable (inbound). The broker never proxies — all
constellation traffic is direct.

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
