# Linux kernel releases — `kernel_releases`

|  |  |
| --- | --- |
| **Module** | [`src/skills/kernel.rs`](../../src/skills/kernel.rs) |
| **Tools** | `kernel_releases` |
| **Network** | keyless API |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Lists the current Linux kernel releases from kernel.org's published
`releases.json`, with no account or key. Each entry shows the moniker (mainline /
stable / longterm), version, release date, EOL status, and source-tarball link, and
the output ends with the latest stable version. Use it to answer "what's the
latest/longterm kernel". Results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `kernel_releases` | — | List current Linux kernel releases: mainline, stable, and longterm versions with release dates, EOL markers, and source-tarball links, plus the latest stable version. Takes no arguments. |

## Configuration & gating
No configuration. The tool is on by default with no tunables; disable it in
`[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)). Kernel
documentation is searchable separately via the `docs_kernel` doc-site provider (see
[`config/02-search.toml`](../../config/02-search.toml)).

## Example uses
- **Find the latest stable kernel** — call `kernel_releases` and read the trailing "latest stable" line.
- **Pick a longterm (LTS) version** — call `kernel_releases` and look for `longterm` monikers that are not marked `[EOL]`.
- **Download a kernel** — call `kernel_releases` to get the version's `source` tarball link, then pass it to `read_pdf`'s sibling fetchers (`fetch_page` / `store_fetch`) or a browser.

## See also
- [tools.md](../tools.md) — full tool reference (Retrieve section).
- [wikipedia.md](wikipedia.md) — keyless reference lookups.
