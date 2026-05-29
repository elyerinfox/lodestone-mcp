# Open access — `unpaywall_lookup`, `openalex_search`, `openalex_work`

|  |  |
| --- | --- |
| **Module** | [`src/skills/openaccess.rs`](../../src/skills/openaccess.rs) |
| **Tools** | `unpaywall_lookup`, `openalex_search`, `openalex_work` |
| **Network** | keyless (Unpaywall, OpenAlex) |
| **Default** | on; gateable via `[tools]` |

## What it does
Finds **legal, openly-licensed** full-text copies of scholarly papers — author
manuscripts, repository/preprint deposits, and publisher open-access — and hands back
a PDF URL you can pipe straight into `read_pdf`. It surfaces only legitimately
open-access copies; it does not bypass paywalls.

- **Unpaywall** (`unpaywall_lookup`) — by DOI, the best OA copy plus all known OA
  locations, with OA status/license/version.
- **OpenAlex** (`openalex_search` / `openalex_work`) — search the open scholarly
  graph or fetch one work; each result carries authors, year, venue, DOI, and the OA
  PDF link when one exists.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `unpaywall_lookup` | `doi` | Best legal OA PDF + all OA locations for a DOI. |
| `openalex_search` | `query`, `max_results?` | Search works; authors/year/venue/DOI + OA PDF. |
| `openalex_work` | `id` | One work by DOI or OpenAlex id, with OA status/PDF. |

## Contact email
Unpaywall's terms require a real contact email — set **`LODESTONE_CONTACT_EMAIL`**
(an `example.com` address is rejected); `unpaywall_lookup` returns a clear error until
it's set. OpenAlex uses the same email for its "polite pool" when present but works
without it.

## Example flow
1. `pubmed_search` / `arxiv_search` / `standards_search` → a DOI.
2. `unpaywall_lookup { doi }` → an OA PDF URL (or "no OA copy").
3. `read_pdf { source: <that URL> }` → the full text.

Or start broad: `openalex_search { query: "..." }` → pick a work → `read_pdf` its
`OA PDF`. For paywalled papers with no OA copy, these tools say so rather than
circumventing the paywall.

## See also
[tools.md](../tools.md) · [pubmed.md](pubmed.md)
