# GitHub metadata — `github_releases` / `github_user` / `github_repo`

|  |  |
| --- | --- |
| **Module** | [`src/skills/github.rs`](../../src/skills/github.rs) |
| **Tools** | `github_releases`, `github_user`, `github_repo` |
| **Network** | keyless GitHub REST API (`api.github.com`) |
| **Default** | on |
| **Config** | none required; optional `[github].token` (shared with the `github` code provider) raises the rate limit. Gate the tools via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml). |

## What it does
Looks up GitHub repository and account metadata through GitHub's public REST API
(`api.github.com`) — no `gh` CLI and no Git clone, just plain HTTPS GETs. It is
keyless: unauthenticated reads work out of the box, and an optional token only
raises the rate limit. All three tools accept a shorthand or a github.com URL and
cache their results.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `github_releases` | `repo`, `max_results?`, `include_prereleases?` | List a repo's releases (newest first): tag, name, published date, link, and release notes. `repo` is `owner/repo` or a github.com URL. `max_results` defaults to 5, capped at 30. `include_prereleases` (default false) adds pre-releases and drafts; otherwise only stable releases are returned. Use for changelogs / "what changed in version X". |
| `github_user` | `user` | Get a user's or org's public profile: type, name, bio, company, location, blog, email, public-repo count, followers/following, join date, and profile URL. `user` is a login, an `@login`, or a github.com/<user> URL. |
| `github_repo` | `repo` | Get a repo's metadata: description, stars, forks, open issues, primary language, topics, license (SPDX), default branch, homepage, last-push date, and archived/fork flags. `repo` is `owner/repo` or a github.com URL. |

## Configuration & gating
No configuration is required. To raise GitHub's rate limit, set a token (classic
or fine-grained, read access is enough) via `[github].token` in config, or the
`LODESTONE_GITHUB_TOKEN` / `GITHUB_TOKEN` environment variable. The token is
shared with the `github` code-search provider. When set, it is sent as a bearer
token; when empty, requests are unauthenticated. Disable the tools through
`[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)).

## Example uses
- **Summarize the latest changes in a project** — `github_repo` with `repo="rust-lang/rust"` to confirm the canonical repo, then `github_releases` (`repo="rust-lang/rust"`, `max_results=3`) to read the most recent release notes.
- **Vet a dependency's project** — `github_repo` with `repo="https://github.com/tokio-rs/tokio"` to check stars, license, and last-push freshness before adopting it.
- **Look up who maintains a repo** — `github_user` with `user="rust-lang"` for the org's profile and public-repo count.
- **Find a specific release** — `github_releases` with `include_prereleases=true` to include drafts/pre-releases when hunting an exact tag.

## See also
- [containers.md](../containers.md) — related keyless container/cloud-native lookups.
- [tools.md](../tools.md) — full tool reference.
