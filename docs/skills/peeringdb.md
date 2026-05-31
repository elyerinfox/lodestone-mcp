# PeeringDB — `peeringdb_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/peeringdb.rs`](../../src/skills/peeringdb.rs) |
| **Tools** | `peeringdb_network`, `peeringdb_ix`, `peeringdb_facility`, `peeringdb_org` |
| **Network** | `peeringdb.com/api` — **keyless** public REST |
| **Default** | **on** |
| **Config** | none |

## What it does

Look up **networks** (ASNs), **internet exchanges** (IXs), **facilities**
(colos), and **organizations** in PeeringDB — the directory the
interconnection industry uses to find each other. Useful for interconnection
planning and figuring out where a given network peers.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `peeringdb_network` | `asn?`, `name?`, `max?` | Network by ASN (preferred) or name substring. |
| `peeringdb_ix` | `name?`, `country?`, `city?`, `max?` | Internet exchanges. |
| `peeringdb_facility` | `name?`, `country?`, `city?`, `max?` | Colo facilities. |
| `peeringdb_org` | `name?`, `max?` | Organizations. |

## Example uses

- **Who is AS13335?** —
  `peeringdb_network { asn: 13335 }` → Cloudflare.
- **IXs in Amsterdam** —
  `peeringdb_ix { city: "Amsterdam" }`.
- **Facilities Cloudflare is in** — `peeringdb_network { asn: 13335 }` then
  drill into its facility list.

## Notes

- **Keyless and public.** Authenticated read access (for private fields)
  isn't currently supported here.
- **Cached.** Results pass through the retrieval cache.

## See also

- [tools.md](../tools.md)
- [skills/osm.md](osm.md) — for physical locations / addresses around a
  facility.
