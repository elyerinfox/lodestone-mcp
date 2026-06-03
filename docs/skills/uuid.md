# uuid — generate / parse / short-form encode

|  |  |
| --- | --- |
| **Module** | [`src/skills/uuid_tools.rs`](../../src/skills/uuid_tools.rs) |
| **Tools** | `uuid_generate`, `uuid_parse`, `uuid_to_short` |
| **Network** | none — pure local compute |
| **Default** | on (no config gate) |

## What it does

Three UUID tools covering generation, introspection, and compact
encoding:

- **`uuid_generate { version, count?, at? }`** — generate v4 (random) or
  v7 (Unix-millisecond timestamp + entropy) UUIDs. v7 is preferred for
  new identifiers in 2025+ because it's sortable by creation time.
  `count` defaults to 1, max 1000. For v7 the optional `at` arg embeds a
  specific RFC3339 timestamp instead of "now"; truncated to millisecond
  precision per RFC 9562 §5.7.
- **`uuid_parse { uuid }`** — parse a UUID and report its version (1-8)
  + variant (RFC 4122/9562, Microsoft, NCS, future) + canonical forms
  (hyphenated, hex-only, urn). For time-based versions (1/6/7) the
  embedded timestamp is decoded and returned as RFC3339 + Unix ms.
- **`uuid_to_short { uuid, encoding }`** — re-encode an existing UUID as
  `base32` (26 chars), `base58` (Bitcoin alphabet, ~22 chars, no
  visually-confusing characters), or `base64url` (22 chars, URL-safe
  alphabet, no padding). Useful for short URLs, file names, anywhere the
  36-char canonical form is too long.

## Why this beats letting the LLM do it

LLMs hallucinate UUIDv7 field layouts roughly all the time — the
millisecond-timestamp prefix, the version nibble position, the variant
bits all get mangled. `uuid_parse` returns the actual decoded
timestamp; `uuid_generate v7 at=<RFC3339>` produces a real, byte-correct
v7 with the requested moment baked in.

## Sources

- RFC 9562 (UUID format / versions 1-8).
- RFC 4648 (base32 / base64url alphabets).
- [bs58 Bitcoin base58 alphabet](https://datatracker.ietf.org/doc/html/draft-msporny-base58-03).

## Example flow

```
1. uuid_generate { version: "v7" }
   → fresh sortable ID with embedded current ms

2. uuid_parse { uuid: "<the uuid>" }
   → version=7, variant=RFC 4122/9562, embedded_timestamp_utc

3. uuid_to_short { uuid: "<the uuid>", encoding: "base64url" }
   → 22-char URL-safe slug
```

## See also

- [`docs/golden-rules.md`](../golden-rules.md) — golden rule 1 (keyless),
  golden rule 12 (citations).
