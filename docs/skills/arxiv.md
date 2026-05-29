# arXiv papers — `arxiv_search` / `arxiv_get`

|  |  |
| --- | --- |
| **Module** | [`src/skills/arxiv.rs`](../../src/skills/arxiv.rs) |
| **Tools** | `arxiv_search`, `arxiv_get` |
| **Network** | keyless API |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Searches and retrieves arXiv preprints without an account or key, using the public
arXiv API (`export.arxiv.org/api/query`, an Atom XML feed). arXiv papers are open
access, so every result includes the free PDF URL — feed that to `read_pdf` to get
the full text. Results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `arxiv_search` | `query`, `max_results?` | Search arXiv by title/abstract/author keywords. Returns title, authors, date, categories, a short abstract, and the abs + free PDF URLs. Default 8 results, capped at 25. |
| `arxiv_get` | `id` | Fetch one paper's metadata and full abstract. `id` accepts `2103.00020`, `arXiv:2103.00020v2`, an abs/pdf URL, or an old-style id like `math/0211159`. Returns title, authors, date, categories, the full abstract, and the PDF URL. |

## Configuration & gating
No configuration. Both tools are on by default with no tunables; disable them in
`[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)).

## Example uses
- **Read a paper in full** — `arxiv_search` for "attention is all you need", then `read_pdf` on the returned PDF URL (e.g. `https://arxiv.org/pdf/1706.03762v5`) for the full text.
- **Pull one known paper's abstract** — `arxiv_get` with `id="2103.00020"` for the full abstract, then `read_pdf` on its PDF URL if you need the body.
- **Survey a topic** — `arxiv_search` with a topic query and a larger `max_results` to scan recent titles, authors, and abstracts.

## See also
- [tools.md](../tools.md) — full tool reference; `read_pdf` extracts a PDF's text locally.
- [huggingface.md](huggingface.md) — models and datasets (HF tags often link arXiv ids).
- [standards.md](standards.md) — published standards via Crossref.
