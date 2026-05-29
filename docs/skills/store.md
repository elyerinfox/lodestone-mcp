# File store & cache — `cache_status` / `store_fetch` / `store_get` / `store_list` / `store_purge`

|  |  |
| --- | --- |
| **Module** | [`src/skills/store.rs`](../../src/skills/store.rs) |
| **Tools** | `cache_status`, `store_fetch`, `store_get`, `store_list`, `store_purge` |
| **Network** | local (disk) + introspection; `store_fetch` reaches the network (local store → constellation peer → source) |
| **Default** | `cache_status` always on; `store_*` off by default (gated by `[store]`) |
| **Config** | [`config/15-store.toml`](../../config/15-store.toml) (`[store]`) |

## What it does
These tools manage the optional on-disk **file store** ([`src/store.rs`](../../src/store.rs), a key-addressed [`FileStore`](../../src/store.rs)) that caches fetched *bytes* — repo files, PDFs, rendered pages — so they can be reused without re-downloading. The store is off by default (it writes to disk); enabling `[store]` exposes the four `store_*` tools. `cache_status` is always available and read-only, reporting the in-memory search/retrieval caches plus the store. Store entries are shared over the constellation: their hashes are advertised in each node's Bloom filter and `store_fetch` resolves a URL as local store → a constellation peer that has it → finally the source (see [constellation.md](../constellation.md)).

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `cache_status` | — | Report the in-memory search cache, retrieval-output cache, and the file store (counts + total size; store dir). Read-only, **always available**. |
| `store_fetch` | `url` | Download `url`'s bytes into the store and return the local path + size. Dodges the source when possible: local copy → a constellation peer that has it → the source. Then read it back with `store_get`. |
| `store_get` | `key`, `max_chars?` | Read a stored entry's content as text (UTF-8 lossy, truncated). `key` is the URL it was stored under. |
| `store_list` | — | List store entries (key/URL, size, age) newest first, plus the constellation seed ratio (↑served/↓fetched) per entry when tracked. |
| `store_purge` | `key?` | Remove one entry by `key`, or purge the whole store when `key` is omitted. |

## Configuration & gating
`cache_status` is never gated — it stays available to report whatever caches exist. The `store_*` tools require `[store].enabled = true` (env `LODESTONE_STORE_ENABLED`); when disabled they return an error pointing at that key. Other `[store]` keys: `dir` (empty = `./.lodestone-store`, env `LODESTONE_STORE_DIR`), `ttl_secs` (entry lifetime, `0` = no expiry, default 86400, env `LODESTONE_STORE_TTL_SECS`), and `max_bytes` (total byte budget, oldest evicted past it, `0` = unbounded, default 536870912, env `LODESTONE_STORE_MAX_BYTES`). Retention runs on write: entries past `ttl_secs` are dropped, then the oldest are evicted until under `max_bytes`. All tools are independently gateable via `[tools]`.

## Example uses
- **Read a rate-limited PDF once across a mesh** — `store_fetch` an arXiv/IETF PDF (served from a constellation peer if one already has it), then `read_pdf` the local copy.
- **Inspect cache state** — `cache_status` to see search/retrieval hit counts and store size + path before deciding whether to re-fetch.
- **Audit and prune the store** — `store_list` to see entries, sizes, ages and per-blob seed ratios, then `store_purge` a stale key (or the whole store).
- **Re-read fetched text** — after a `store_fetch`, `store_get` the same URL key to pull its text back without another download.

## See also
[constellation.md](../constellation.md), [tools.md](../tools.md)
