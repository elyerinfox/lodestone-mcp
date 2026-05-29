# Published standards — `standards_search`

|  |  |
| --- | --- |
| **Module** | [`src/skills/standards.rs`](../../src/skills/standards.rs) |
| **Tools** | `standards_search` |
| **Network** | keyless API (Crossref) |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Looks up published standards and specifications by title through the keyless
Crossref API (`api.crossref.org/works`), covering IEEE, SAE, NIST, ISO, ANSI, IEC,
and more. Results are filtered to standards/reports/monographs so journal noise is
dropped, with an optional publisher filter. This returns metadata plus a `doi.org`
link only — IEEE and SAE full text is paywalled; NIST publications are free, so
pair them with `read_pdf` on the linked PDF. Results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `standards_search` | `query`, `publisher?`, `max_results?` | Search standards by title (e.g. "IEEE 802.11", "SAE J1939", "NIST 800-53", "ISO 26262", or a topic). `publisher` narrows to one body — `ieee`, `sae`, `nist`, `iso`, `ansi`, `iec`, or any substring of the publisher name. Returns title, publisher, type, year, DOI, and a doi.org link. Default 8 results, capped at 25. |

## Configuration & gating
No configuration. The tool is on by default with no tunables; disable it in
`[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)). Note that the
related `docs_nist` doc-site provider (see [`config/02-search.toml`](../../config/02-search.toml))
is a separate path to NIST material.

## Example uses
- **Read a free NIST publication** — `standards_search` with `query="800-53"`, `publisher="nist"` to get the DOI/URL, then `read_pdf` on the linked PDF for the full document.
- **Locate a paywalled standard** — `standards_search` for "ISO 26262" with `publisher="iso"` to obtain the official metadata and DOI (full text is not available).
- **Find an automotive spec** — `standards_search` for "SAE J1939" to list matching SAE documents with years and DOIs.

## See also
- [tools.md](../tools.md) — full tool reference (Retrieve section).
- [rfc.md](rfc.md) — IETF RFCs (full text, keyless).
- [arxiv.md](arxiv.md) — open-access papers (full PDFs via `read_pdf`).
