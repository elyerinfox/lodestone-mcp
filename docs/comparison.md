# How Lodestone compares

Lodestone overlaps with several tools but targets a specific niche: **keyless,
code-aware, MCP-native, self-hosted.**

| | lodestone | SearXNG | Brave/Tavily/Exa MCP | `fetch` MCP | Firecrawl | GitHub MCP |
| --- | --- | --- | --- | --- | --- | --- |
| API key required | No¹ | No | **Yes** | No | **Yes** | **Yes** (token) |
| MCP-native | **Yes** | No (needs wrapper) | Yes | Yes | Yes | Yes |
| Web search | Yes (2 engines) | **Yes (~200 engines)** | Yes | No | partial | No |
| Code/forge search | **Yes** (GH/GL/Gitea) | No | No | No | No | GitHub only |
| Raw file retrieval | **Yes** | No | No | partial | Yes | Yes |
| Q&A (StackExchange) | **Yes** | via engines | No | No | No | No |
| Docs & registries | **Yes** (registries + framework docs) | via engines | No | No | No | No |
| Containers / cloud-native | **Yes** (Docker Hub, OCI, Artifact Hub) | No | No | No | No | No |
| Headless JS render | **On demand** | No | n/a (hosted) | No | Yes | n/a |
| Archive fallback | **Yes** | No | No | No | No | No |
| Self-hosted / offline-friendly | Yes (single binary) | Yes (Python+Redis) | No (SaaS) | Yes | No (SaaS) | partial |
| Result breadth / ranking | **Strong** (composite: RRF + consensus + relevance + authority + diversity) | Strong | Strong | n/a | Strong | n/a |

¹ Optional GitHub token for authenticated code search; everything else keyless.

## When to prefer something else

- **SearXNG** — you want the broadest, best-ranked general web search and don't
  mind running Python + Redis and wrapping it for MCP. (Lodestone can even use a
  SearXNG instance as a provider.) Lodestone trades breadth for being code-aware,
  MCP-native, and a single binary.
- **Brave / Tavily / Exa MCP** — you're fine with an API key and want managed,
  high-quality search/answers. Lodestone's pitch is *no key*.
- **Firecrawl** — you need robust large-scale crawling/extraction. Lodestone's
  rendering is single-page and best-effort.
- **Official GitHub MCP** — you live in GitHub (issues/PRs/repos) with a token.
  Lodestone is multi-forge and keyless-first, focused on *search + read*.

## Honest limitations

Scraping is brittle and breaks when sites change markup; DuckDuckGo/Google
aggressively rate-limit or CAPTCHA datacenter IPs; the StackExchange keyless API
has a daily quota; the headless browser adds latency and a Chrome dependency.
Lodestone leans on fallback chains and the web archive to stay useful despite this.
