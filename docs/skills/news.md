# News feeds — `news_feed`

|  |  |
| --- | --- |
| **Module** | [`src/skills/news.rs`](../../src/skills/news.rs) |
| **Tools** | `news_feed` |
| **Network** | keyless (any public RSS/Atom feed) |
| **Default** | on (read-only); gateable via `[tools]` |

## What it does
Fetches a syndication feed and returns its recent items — title, link, date, and a
short text summary. Handles both **RSS 2.0** (`<item>`) and **Atom** (`<entry>`)
feeds (parsed with `roxmltree`, CDATA/HTML bodies flattened to text). Read-only,
keyless, and cached. Generalizes the Medium tag-RSS provider to arbitrary feeds.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `news_feed` | `source`, `max_results?` | Recent items from an RSS/Atom feed. |

`source` is either a full feed URL (`https://…`) or a built-in shorthand:
`hackernews` (`hn`), `bbc`, `theverge` (`verge`), `arstechnica` (`ars`),
`lobsters`, `lwn`. `max_results` defaults to 15 (capped at 50).

## Example uses
- **Tech headlines** — `news_feed hackernews` for the current HN front page.
- **A specific outlet** — `news_feed https://feeds.bbci.co.uk/news/technology/rss.xml`.
- **A blog/changelog** — point it at any project's Atom feed to summarize what's new.
- **Combine with retrieval** — pick an item, then `fetch_page` its link for the full text.

## Notes
- Feeds vary in how much body they include; the summary is whatever the feed
  provides (often a teaser), truncated. Use `fetch_page` on the link for the full article.
- Not a search engine — it returns the feed's *current* items, newest first.

## See also
[tools.md](../tools.md)
