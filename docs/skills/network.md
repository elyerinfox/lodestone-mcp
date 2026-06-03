# network — CIDR / subnet math + IP classification

|  |  |
| --- | --- |
| **Module** | [`src/skills/network.rs`](../../src/skills/network.rs) |
| **Tools** | `net_cidr_info`, `net_cidr_subnets`, `net_ip_in_cidr`, `net_ip_classify`, `net_cidr_summarize` |
| **Network** | none — pure local compute |
| **Default** | on (no config gate; pure compute) |

## What it does

Five tools covering the binary subnet arithmetic LLMs reliably get wrong:

- **`net_cidr_info { cidr }`** — decompose a CIDR (v4 or v6) into network
  address, broadcast (v4 only), netmask, wildcard mask, host range, total
  addresses, usable hosts, and prefix length. Off-aligned hosts (e.g.
  `10.42.7.83/19`) are truncated to the canonical network in the output.
- **`net_cidr_subnets { cidr, new_prefix, limit? }`** — split a CIDR into
  equal-sized subnets at a longer prefix. A /24 with `new_prefix=26`
  yields four /26s. `limit` (default 256) bounds the response so a
  /16 → /32 split doesn't dump 65 536 rows.
- **`net_ip_in_cidr { ip, cidr }`** — membership test plus the 0-indexed
  position of the IP within the block when contained.
- **`net_ip_classify { ip }`** — IANA Special-Purpose Address Registry
  classification: public / private / loopback / link-local / multicast /
  CGNAT / documentation / unspecified / reserved (v4) / unique-local /
  TEREDO / 6to4 (v6). Each category cites the governing RFC.
- **`net_cidr_summarize { cidrs }`** — coalesce a list of CIDRs into the
  minimal covering set. IPv4 and IPv6 inputs are aggregated separately.

All five are pure-compute and run locally — no DNS, no registry, no
external dependency at runtime.

## Configuration & gating

This is a pure-compute family and has no config section. Individual tools
can still be disabled via the global `[tools].disabled` allow/deny list.

## Sources

- RFC 791 (IPv4), RFC 4632 (CIDR), RFC 1918 (private IPv4 ranges).
- RFC 6598 (CGNAT 100.64.0.0/10).
- RFC 5737 / RFC 3849 (documentation prefixes).
- RFC 4291 (IPv6 addressing), RFC 4193 (unique local v6), RFC 4380 (TEREDO),
  RFC 3056 (6to4).
- [IANA Special-Purpose Address Registries](https://www.iana.org/assignments/iana-ipv4-special-registry/)
  for IPv4 and IPv6, pinned to the May 2024 snapshot.

## Example flow

```
1. net_ip_classify { ip: "10.42.7.83" }
   → category=private, rfc=1918

2. net_cidr_info { cidr: "10.42.7.83/19" }
   → network=10.42.0.0/19, broadcast=10.42.31.255, 8190 usable hosts

3. net_cidr_subnets { cidr: "10.42.0.0/19", new_prefix: 22 }
   → 8 × /22 subnets for per-team subdivision

4. net_cidr_summarize { cidrs: ["10.42.0.0/22", "10.42.4.0/22"] }
   → summarized=["10.42.0.0/21"] — confirm the route you'd advertise upstream
```

## See also

- [`docs/golden-rules.md`](../golden-rules.md) — golden rule 1 (keyless),
  golden rule 9 (one method per tool), golden rule 12 (citations).
- [`docs/tools.md`](../tools.md) for the per-tool argument table.
