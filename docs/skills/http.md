# http — status, header, and cache-policy decoders

|  |  |
| --- | --- |
| **Module** | [`src/skills/http_decode.rs`](../../src/skills/http_decode.rs) |
| **Tools** | `http_status_decode`, `http_header_explain`, `http_cache_decode` |
| **Network** | none — pure local compute |
| **Default** | on (no config gate) |

## What it does

Three deterministic answers for HTTP semantics LLMs typically mix up:

- **`http_status_decode { code }`** — RFC 9110 name, class, semantics,
  cacheability hint, and LLM-typical gotcha for any 1xx-5xx code.
  Special focus on the 301/302/303/307/308 redirect family and
  429/503 retry semantics.
- **`http_header_explain { name }`** — purpose, request/response
  context, syntax, and gotchas for ~25 well-known headers. Curated
  from RFC 9110 / 9111 / 6265bis / 7234 / 7239 / 8297 plus the IANA
  HTTP Field Name Registry.
- **`http_cache_decode { cache_control?, expires?, pragma?, vary?, age? }`** —
  parse a response's cache-related headers into a structured verdict:
  storable? shared-cacheable? must-revalidate every use?
  effective max-age? which `Vary` axes apply? Each `Cache-Control`
  directive (`public`, `private`, `no-cache`, `no-store`,
  `must-revalidate`, `proxy-revalidate`, `immutable`,
  `stale-while-revalidate`, `stale-if-error`, `s-maxage`) is surfaced.

## Sources

- RFC 9110 (HTTP Semantics).
- RFC 9111 (HTTP Caching).
- RFC 5861 (stale-while-revalidate, stale-if-error).
- RFC 6265bis (cookies).
- RFC 7239 (Forwarded:).
- IANA HTTP Status Code + Field Name Registries, May 2024 snapshot.

## Example flow

```
1. http_status_decode { code: 308 }
   → Permanent Redirect. Method preserved (vs 301 which rewrites POST→GET).

2. http_header_explain { name: "Vary" }
   → Cache-keying semantics; gotcha about gzipped responses leaking.

3. http_cache_decode { cache_control: "public, max-age=600", vary: "Accept-Encoding" }
   → Storable, shared-cacheable, fresh for 600s, two cache-key axes.
```

## See also

- [`docs/golden-rules.md`](../golden-rules.md) — golden rule 1 (keyless),
  golden rule 12 (citations).
