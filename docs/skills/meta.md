# Meta & constellation introspection — `list_providers` / `constellation_status` / `constellation_peers` / `constellation_seeds`

|  |  |
| --- | --- |
| **Module** | [`src/skills/meta.rs`](../../src/skills/meta.rs) |
| **Tools** | `list_providers`, `constellation_status`, `constellation_peers`, `constellation_seeds` |
| **Network** | introspection (read server state only) |
| **Default** | on |
| **Config** | none (the `constellation_*` views reflect [`config/06-network.toml`](../../config/06-network.toml) `[network]`) |

## What it does
These read-only tools report server state. `list_providers` shows the active search providers and the order/strategy/ranking they are tried in. The three `constellation_*` tools surface the peer-to-peer [constellation](../constellation.md): `constellation_status` is the full graph (this node's id, every known peer's reputation/reachability, and the mesh edges they advertise), `constellation_peers` lists reachable nodes by hop distance (direct peers = 1 hop), and `constellation_seeds` shows BitTorrent-style per-blob seed accounting. The three `constellation_*` tools report a disabled notice when `[network].enabled` is false.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `list_providers` | — | List the configured search providers and the order they are tried, for web, code and Q&A — to check which sources are active. |
| `constellation_status` | — | The constellation graph: this node's id and its known peers with reputation, reachability, miss count, and the mesh edges each advertised. Disabled-notice when off. |
| `constellation_peers` | — | Constellation nodes in reach and how many **hops** away each is (direct peers = 1; nodes only reachable via a peer's advertised list = 2+), with each direct peer's stable machine id, reputation, and reachability. Disabled-notice when off. |
| `constellation_seeds` | — | Per-blob **seed ratio**: for each shared file/page hash, bytes this node served to peers vs. fetched from them, and the served/fetched ratio. Disabled-notice when off. |

## Configuration & gating
None of these take configuration — they just read live state. The `constellation_*` reports reflect the running `[network]` config ([`config/06-network.toml`](../../config/06-network.toml), env `LODESTONE_NETWORK_*`); when `[network].enabled` is false they print a short disabled notice rather than a graph. Seed data appears only when the on-disk file store (`[store]`) is enabled, since blob sharing rides on store entry hashes (see [constellation.md](../constellation.md)). All four tools are independently gateable via `[tools]`.

## Example uses
- **Confirm which sources are live** — `list_providers` to see the active providers, strategy, and ranking after a config change.
- **Check mesh reach** — `constellation_peers` to see how many nodes are reachable and at what hop distance (direct = 1) before relying on constellation-served results.
- **Debug peer trust** — `constellation_status` to inspect per-peer reputation, reachability, and advertised edges when a query isn't being served from the mesh.
- **Track sharing fairness** — `constellation_seeds` to see per-blob served/fetched ratios (who you're seeding vs. leeching from).

## See also
[constellation.md](../constellation.md), [tools.md](../tools.md)
