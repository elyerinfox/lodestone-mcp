# PubMed & NCBI — `pubmed_search`, `pubmed_summary`, `ncbi_search`, `ncbi_summary`

|  |  |
| --- | --- |
| **Module** | [`src/skills/pubmed.rs`](../../src/skills/pubmed.rs) |
| **Tools** | `pubmed_search`, `pubmed_summary`, `ncbi_search`, `ncbi_summary` |
| **Network** | keyless (NCBI E-utilities; optional `LODESTONE_NCBI_API_KEY`) |
| **Default** | on; gateable via `[tools]` |

## What it does
Queries NCBI through its public **E-utilities** API (`esearch` → `esummary` →
`efetch`) — the single interface behind **ncbi.nlm.nih.gov** (the same one
`Bio.Entrez` uses). No API key required; set `LODESTONE_NCBI_API_KEY` to raise the
rate limit. Results are cached.

- **PubMed** tools are convenient shortcuts for the biomedical literature (`db=pubmed`),
  with author/journal/DOI formatting and a real abstract via `efetch`. Links point at
  `https://pubmed.ncbi.nlm.nih.gov/<pmid>/`.
- **NCBI** tools are generic across **any** Entrez database via a `db` parameter —
  `pmc`, `gene`, `protein`, `nucleotide`, `snp`, `clinvar`, `taxonomy`, `books`,
  `mesh`, `assembly`, `genome`, … Links point at `https://www.ncbi.nlm.nih.gov/<db>/<uid>/`.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `pubmed_search` | `query`, `max_results?` | PubMed papers: PMID, title, authors, journal, date, link. |
| `pubmed_summary` | `pmid`, `max_chars?` | One paper's citation + DOI + abstract text. |
| `ncbi_search` | `db`, `query`, `max_results?` | Search any NCBI database; UIDs + headline + key fields + link. |
| `ncbi_summary` | `db`, `id` | One record's summary fields + ncbi.nlm.nih.gov link. |

- PubMed `query` accepts free text **or** field tags, e.g. `crispr off-target`,
  `asthma[Title]`, `smith j[Author] AND 2023[Date - Publication]`.
- `max_results` defaults to 10 (capped at 50); `max_chars` caps the abstract length
  (default 3000).

## Example uses
- **Literature scan** — `pubmed_search { query: "GLP-1 weight loss", max_results: 20 }`.
- **Read an abstract** — `pubmed_summary { pmid: "38000000" }`.
- **A gene** — `ncbi_search { db: "gene", query: "BRCA1 human" }` → `ncbi_summary { db: "gene", id: "672" }`.
- **A sequence** — `ncbi_search { db: "nucleotide", query: "spike protein SARS-CoV-2" }`.
- **Taxonomy** — `ncbi_search { db: "taxonomy", query: "Homo sapiens" }`.

## See also
[tools.md](../tools.md)
