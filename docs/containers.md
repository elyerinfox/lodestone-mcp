# Containers & cloud-native

Keyless tools for inspecting container images and the Kubernetes ecosystem. All
are read-only and require no account — Docker Hub and Artifact Hub expose plain
JSON APIs, and OCI-registry access uses the Distribution Spec's **anonymous**
bearer-token flow (the same one `docker pull` uses for public images: a `401`
`WWW-Authenticate: Bearer realm=…` challenge → fetch a token from that realm → retry).

Code: [`src/oci.rs`](../src/oci.rs) (Docker Hub + OCI distribution),
[`src/artifacthub.rs`](../src/artifacthub.rs). Each is an independent skill,
gateable via `[tools]`.

## Docker Hub

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `docker_search` | `query`, `max_results?` | Search Docker Hub images — name, official/verified, stars, pulls, description. |
| `docker_image` | `image` | One repository's details: description, stars, pulls, last-updated, long description. |
| `docker_tags` | `image`, `max_results?` | List tags (newest first) with compressed size, last-pushed date, architectures. |

`image` accepts `nginx` (official → `library/nginx`), `bitnami/redis`, etc. An
optional `:tag` is ignored by `docker_image`/`docker_tags` (they describe the repo).

## Any OCI registry

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `oci_tags` | `reference`, `max_results?` | List an image's tags on **any** OCI registry. |
| `oci_manifest` | `reference` | Inspect a manifest: multi-arch platforms, or layer count + total size + config digest. |

`reference` works across registries: `nginx`, `nginx:1.27`,
`ghcr.io/owner/image`, `quay.io/ns/repo`, `localhost:5000/team/app`, and
`…@sha256:<digest>`. The registry host is detected from the first segment when it
looks like a domain or has a port; otherwise the reference resolves to Docker Hub.

## Kubernetes ecosystem (Artifact Hub)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `artifacthub_search` | `query`, `kind?`, `max_results?` | Search [Artifact Hub](https://artifacthub.io): Helm charts, Operators, krew plugins, Falco/OPA/Kyverno/Gatekeeper policies, Tekton tasks, … |

`kind` optionally filters by package kind: `helm`, `olm`, `krew`, `falco`, `opa`,
`kyverno`, `gatekeeper`, `tekton-task`, `coredns`, `container`, and more. Results
link to the package page on artifacthub.io.

## Documentation search

The framework-docs family also covers tooling docs: `docs_docker`
(docs.docker.com), `docs_kubernetes` (kubernetes.io), and `docs_helm` (helm.sh)
are on by default and join `docs_search`. See
[providers/frameworks.md](providers/frameworks.md).
