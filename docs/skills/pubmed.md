# PubMed — `pubmed_search`, `pubmed_summary`

|  |  |
| --- | --- |
| **Module** | [`src/skills/pubmed.rs`](../../src/skills/pubmed.rs) |
| **Tools** | `pubmed_search`, `pubmed_summary` |
| **Network** | keyless (NCBI E-utilities; optional `LODESTONE_NCBI_API_KEY`) |
| **Default** | on; gateable via `[tools]` |

## What it does
Searches the biomedical literature on **PubMed** and reads abstracts, via NCBI's
public **E-utilities** API (`esearch` → `esummary` → `efetch`) — the same interface
`Bio.Entrez` uses. No API key required; set `LODESTONE_NCBI_API_KEY` in the
environment to raise the rate limit. Results are cached. Links point at
`https://pubmed.ncbi.nlm.nih.gov/<pmid>/`.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `pubmed_search` | `query`, `max_results?` | Find papers: PMID, title, authors, journal, date, link. |
| `pubmed_summary` | `pmid`, `max_chars?` | One paper's citation + DOI + abstract text. |

- `query` accepts free text **or** PubMed field tags, e.g. `crispr off-target`,
  `asthma[Title]`, or `smith j[Author] AND 2023[Date - Publication]`.
- `max_results` defaults to 10 (capped at 50); `max_chars` caps the abstract length
  (default 3000).

## Example uses
- **Literature scan** — `pubmed_search { query: "GLP-1 weight loss", max_results: 20 }`.
- **Read an abstract** — `pubmed_summary { pmid: "38000000" }`.
- **Targeted** — `pubmed_search { query: "BRCA1[Title] AND review[Publication Type]" }`.

## See also
[tools.md](../tools.md)
