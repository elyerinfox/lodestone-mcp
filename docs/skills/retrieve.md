# Retrieve — `fetch_page` / `render_page` / `webpage_to_pdf` / `read_pdf` / `fetch_repo_file`

|  |  |
| --- | --- |
| **Module** | [`src/skills/retrieve.rs`](../../src/skills/retrieve.rs) |
| **Tools** | `fetch_page`, `render_page`, `webpage_to_pdf`, `read_pdf`, `fetch_repo_file` |
| **Network** | keyless web scrape / headless browser (`render_page`, `webpage_to_pdf`); `read_pdf` also reads local paths |
| **Default** | on |
| **Config** | [`config/00-server.toml`](../../config/00-server.toml) (`[retrieval]`) |

## What it does
Retrieval tools fetch one already-identified resource — typically a hit from a search tool. `fetch_page` reads a page over plain HTTP and returns readable text (HTML stripped, PDFs text-extracted); `render_page` does the same through a real headless browser for JS-heavy/SPA or blocked pages. `webpage_to_pdf` saves a rendered page as a local PDF, `read_pdf` extracts a PDF's text layer locally, and `fetch_repo_file` returns a repository file's full contents across GitHub/GitLab/Gitea without a token. Text output is truncated to a character budget; results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `fetch_page` | `url`, `max_chars?` | Page → readable text over plain HTTP. The default reader; if empty/blocked try `render_page`, if the page is gone try `wayback_fetch`. |
| `render_page` | `url`, `max_chars?` | Page → readable text via a headless browser (executes JS). Slower; needs a local Chrome/Chromium. |
| `webpage_to_pdf` | `url`, `path?` | Render a page to a local PDF via the headless browser; writes to `path` or a temp file and returns the saved path + byte count. |
| `read_pdf` | `source`, `max_chars?` | Extract a PDF's text locally. `source` is an absolute URL or a local file path. Scanned/image-only PDFs (no text layer) return an error. |
| `fetch_repo_file` | `target`, `start_line?`, `end_line?` | Full contents of a repo file. `target` is a GitHub/GitLab/Gitea blob URL, a raw URL, or a GitHub `owner/repo/path` shorthand (`main`/`master` tried); an inline `#L10-L40` fragment is honored, and `start_line`/`end_line` override it. |

## Configuration & gating
[`config/00-server.toml`](../../config/00-server.toml) `[retrieval]` sets `default_chars` (returned when a call omits `max_chars`; env `LODESTONE_RETRIEVAL_DEFAULT_CHARS`) and `max_chars` (the hard cap any call is clamped to; env `LODESTONE_RETRIEVAL_MAX_CHARS`). When text is cut off it ends with a `[... truncated ...]` marker — call again with a larger `max_chars`. Each tool is gateable via `[tools]`. `render_page` and `webpage_to_pdf` require a local Chrome/Chromium (see [`crate::browser`](../../src/browser.rs)).

For `read_pdf` with a URL, bytes are fetched via the shared file store / constellation path (`fetch_bytes_shared`): the local file store is checked first, then a constellation peer, then the source. A PDF a peer has already cached (arXiv, IETF, …) serves the mesh, so every node need not re-hit a rate-limited source. See [constellation.md](../constellation.md) and the file store ([`config/15-store.toml`](../../config/15-store.toml)).

## Example uses
- **Read a search result** — `web_search` or `docs_search`, then `fetch_page` on the chosen URL; if it comes back empty (SPA), retry with `render_page`.
- **Read source for a code hit** — `code_search`, then `fetch_repo_file` on the result URL, optionally with `start_line`/`end_line` to view just the relevant span.
- **Read a paper** — `arxiv_search`/`arxiv_get` for the PDF URL, then `read_pdf` (shared via the store/constellation so peers reuse the bytes).
- **Snapshot a page for the record** — `webpage_to_pdf` to render and save a PDF, then `read_pdf` on that path to extract its text.

## See also
[tools.md](../tools.md), [search.md](search.md), [archive.md](archive.md), [constellation.md](../constellation.md)
