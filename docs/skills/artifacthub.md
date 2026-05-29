# Artifact Hub — `artifacthub_search`

|  |  |
| --- | --- |
| **Module** | [`src/skills/artifacthub.rs`](../../src/skills/artifacthub.rs) |
| **Tools** | `artifacthub_search` |
| **Network** | keyless Artifact Hub JSON API (`artifacthub.io/api/v1`) |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Searches [Artifact Hub](https://artifacthub.io), the CNCF index of
Kubernetes-ecosystem packages: Helm charts, Operators (OLM), krew kubectl plugins,
Falco rules, OPA/Kyverno/Gatekeeper policies, Tekton tasks, and more. It is keyless
and plain-HTTP — a single `GET /api/v1/packages/search`, no account or key. Each hit
is rendered with its name, version, package kind, publisher, star count, description,
and a link to the package page on artifacthub.io. Results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `artifacthub_search` | `query`, `kind?`, `max_results?` | Full-text search across Artifact Hub packages. `query` is a chart/operator/plugin name or keyword. `kind` optionally restricts to one package-kind slug — `helm`, `olm`, `krew`, `falco`, `opa`, `kyverno`, `gatekeeper`, `tekton-task`, `coredns`, `container`, and more (omit to search all kinds; an unknown slug is ignored). `max_results` defaults to 10, capped at 30. |

## Configuration & gating
No configuration. The tool is on by default with no tunables; disable it through
`[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)).

## Example uses
- **Find a Helm chart** — `artifacthub_search` with `query="ingress-nginx"`, `kind="helm"` to list matching charts with versions, publishers, and links.
- **Discover then deploy** — `artifacthub_search` with `query="prometheus"`, `kind="helm"` to locate a chart, then hand the resulting kubefile/manifest to the Kubernetes cluster tools (`k8s_apply`) for installation.
- **Browse policies** — `artifacthub_search` with `query="pod security"`, `kind="kyverno"` to find Kyverno policy packages.
- **Search every kind** — `artifacthub_search` with just `query="falco"` to span charts, rules, and operators at once.

## See also
- [containers.md](../containers.md) — overview of container/cloud-native tools, including the Kubernetes cluster and image-registry lookups.
- [tools.md](../tools.md) — full tool reference.
