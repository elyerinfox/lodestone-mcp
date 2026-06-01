# SSRF guard for delegated browser work (infrastructure, not a tool)

|  |  |
| --- | --- |
| **Module** | [`src/skills/ssrf.rs`](../../src/skills/ssrf.rs) |
| **Tools** | none directly — applied inside the browser-session manager when a session is marked `restrict_to_public: true` |
| **Network** | DNS lookups via the system resolver (for hostnames the guard can't decide synchronously) |
| **Default** | armed on every **guest session** (peer-hosted), bypassed on every **local persona** (model-owned) |

## What it does

Refuses any URL whose host resolves to the host's local network when
called on a session a constellation peer is driving on our behalf.
The classic SSRF threat: a peer using our browser to enumerate our
LAN, hit private admin panels, or fingerprint cloud-metadata
endpoints (169.254.169.254). The local model's own
`browser_persona_get` / `browser_open` calls are not subject to this
guard — the operator opted into local tool use, so the constraint
would be noise.

## What it refuses

| Range | Why |
| --- | --- |
| `127.0.0.0/8` | Loopback. |
| `0.0.0.0/8` | "This network." |
| `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` | RFC1918 private. |
| `169.254.0.0/16` | Link-local — includes EC2 / GCE / Azure / DO cloud-metadata at `169.254.169.254`. |
| `100.64.0.0/10` | CGNAT. Conservative — remove the check if your deployment uses CGNAT addresses for production. |
| `192.0.0.0/24` | IETF protocol assignments. |
| `192.0.2.0/24` | TEST-NET-1. |
| `198.18.0.0/15` | Benchmarking. |
| `198.51.100.0/24` | TEST-NET-2. |
| `203.0.113.0/24` | TEST-NET-3. |
| `224.0.0.0/4` | Multicast. |
| `240.0.0.0/4` | Reserved. |
| `::1` | IPv6 loopback. |
| `fc00::/7` | IPv6 ULA. |
| `fe80::/10` | IPv6 link-local. |
| `100::/64` | Discard (RFC 6666). |
| Multicast / unspecified | Always refused. |
| IPv4-mapped IPv6 | Defers to the IPv4 check. |
| `.local`, `.lan`, `.internal`, `.home.arpa`, `.test`, `.invalid`, `.localhost`, `.intranet` | TLDs known to resolve only on local networks. Refused without DNS — saves a round-trip *and* closes a TOCTOU window where a poisoned resolver could swap private for public between check and connect. |

Unsupported schemes (`file:`, `chrome:`, anything other than `http:` /
`https:`) are also refused. `about:blank` is allowed as a special
case so the manager can return a restricted session to a known-empty
state without hitting DNS.

## How it decides

```
parse URL
├─ scheme not http/https           → refuse
├─ host matches a local TLD        → refuse (no DNS — TOCTOU-safe)
├─ host is a literal IP
│  └─ in a private/reserved range  → refuse (synchronous)
│     else                         → allow
└─ host is a name
   └─ resolve via system DNS
      └─ ANY resolved address is private → refuse (Chromium would fall
                                                    back among them; one
                                                    poisons the set)
         else                            → allow
```

Synchronous checks first means literal IPs decide without a DNS
round-trip. Hostnames pay one resolve call.

## Where the lever is

`[network.capabilities].browser`:

- `false` (default) — the `/constellation/browser_persona` endpoint
  itself refuses with `403 disabled`. The SSRF guard never runs
  because no guest session ever gets created.
- `true` — peers can ask us to drive guest sessions. The session
  manager marks them `restrict_to_public: true` and every navigation
  routes through this module. `browser_eval` is rejected outright on
  restricted sessions (raw `fetch()` would bypass URL guards).

The cap can be flipped at runtime from the constellation settings
drawer — no restart.

## Why `browser_eval` is the carve-out

The URL guard is enforced at the `goto()` and post-action-URL re-check
layer. JavaScript running in the page can issue arbitrary `fetch(...)`
calls that the URL guard doesn't see — they happen inside Chromium.
The conservative policy is "no `browser_eval` on restricted sessions";
click / type / extract / wait remain the navigation surface.

A future hardening pass (tracked in
[`docs/security.md`](../security.md#open-security-tasks)) will add a
CDP `Network.setRequestInterception` filter that refuses
private-network sub-requests at the network layer, which would let
`browser_eval` come back on safely.

## Tests

`src/skills/ssrf.rs` ships unit tests for: loopback v4 + v6,
RFC1918, link-local + cloud-metadata, local TLDs, unsupported
schemes, `about:blank` passthrough, public-v4 acceptance. They run
under the standard `cargo test`.

## See also

- [`docs/security.md`](../security.md#browser-sandbox) — full audit
  reference for the browser sandbox, including the SSRF policy in
  context of the other browser controls.
- [`docs/skills/browser_session.md`](browser_session.md) — operator-
  and model-facing reference for sessions / personas / guest
  sessions, including the runtime lever for hosting guests.
- [`docs/constellation.md`](../constellation.md#capabilities) — the
  per-feature opt-in set this guard is the security half of.
