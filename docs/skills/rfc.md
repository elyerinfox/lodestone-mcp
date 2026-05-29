# IETF RFCs — `rfc_get` / `rfc_search`

|  |  |
| --- | --- |
| **Module** | [`src/skills/rfc.rs`](../../src/skills/rfc.rs) |
| **Tools** | `rfc_get`, `rfc_search` |
| **Network** | keyless API |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Retrieves IETF RFCs without an account or key. `rfc_get` fetches an RFC's full
text directly from the RFC Editor (`rfc-editor.org/rfc/rfcN.txt`); `rfc_search`
finds RFCs by title through the IETF Datatracker's JSON document API. Results are
cached in the retrieval cache.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `rfc_get` | `document`, `max_chars?` | Fetch one RFC's full text. `document` is a number or `rfc`-prefixed id (`9110`, `rfc9110`, `RFC 9110`). RFCs are long, so the output is truncated to a character budget — raise `max_chars` (or call again) to read further. |
| `rfc_search` | `query`, `max_results?` | Search RFCs by words in the title (e.g. "http semantics", "tls") via the Datatracker. Returns RFC number, title, and abstract. Default 10 results, capped at 25. |

## Configuration & gating
No configuration. Both tools are on by default and have no tunables; disable them
in `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)) like any other
tool.

## Example uses
- **Read a spec end to end** — `rfc_search` for "http semantics" to find RFC 9110, then `rfc_get` with `document=9110` and a large `max_chars` to read the full text.
- **Jump straight to a known RFC** — `rfc_get` with `document="RFC 791"` for the IP spec.
- **Discover related RFCs** — `rfc_search` for "tls" to list the TLS RFCs with their abstracts.

## See also
- [tools.md](../tools.md) — full tool reference (Retrieve section).
- [standards.md](standards.md) — published standards (IEEE/SAE/NIST/ISO) via Crossref.
