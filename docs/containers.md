# Containers & cloud-native

Keyless tools for inspecting container images and the Kubernetes ecosystem. All
are read-only and require no account — Docker Hub and Artifact Hub expose plain
JSON APIs, and OCI-registry access uses the Distribution Spec's **anonymous**
bearer-token flow (the same one `docker pull` uses for public images: a `401`
`WWW-Authenticate: Bearer realm=…` challenge → fetch a token from that realm → retry).

Code: [`src/skills/oci.rs`](../src/skills/oci.rs) (Docker Hub + OCI distribution),
[`src/skills/artifacthub.rs`](../src/skills/artifacthub.rs). Each is an independent skill,
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

## Local Docker daemon (write access)

A **local-system** capability, separate from the keyless web tools above:
lodestone talks to your Docker daemon directly via the Engine API over the platform
socket (Windows named pipe / unix socket; honors `DOCKER_HOST`) — no `docker` CLI.
Code: [`src/skills/docker.rs`](../src/skills/docker.rs). Gated by `[docker]` (see
[`config/08-docker.toml`](../config/08-docker.toml)); `enabled` on by default.
Destructive actions are always exposed but require a per-call confirmation token
(`confirm`/`trust` args — see [tools.md](tools.md#confirming-destructive-actions));
`allow_destructive` pre-authorizes them. Each action is its own tool.

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `docker_ps` | `all?` | read | List containers (running, or all). |
| `docker_images` | — | read | List local images. |
| `docker_inspect` | `container` | read | Full JSON for a container. |
| `docker_logs` | `container`, `tail?` | read | A container's stdout/stderr. |
| `docker_info` | — | read | Daemon version + state summary. |
| `docker_pull` | `image` | write | Pull an image onto the daemon. |
| `docker_run` | `image`, `name?`, `command?` | write | Create + start a container. |
| `docker_start` | `container` | write | Start a stopped container. |
| `docker_build` | `context`, `tag`, `dockerfile?` | write | Build an image from a context directory. |
| `docker_stop` | `container`, `confirm?`, `trust?` | **destructive** | Stop a running container (confirm first). |
| `docker_remove` | `container`, `force?`, `confirm?`, `trust?` | **destructive** | Remove a container (confirm first). |
| `docker_exec` | `container`, `command`, `confirm?`, `trust?` | **destructive** | Run a command inside a container (confirm first). |
| `docker_rmi` | `image`, `force?`, `confirm?`, `trust?` | **destructive** | Remove an image (confirm first). |

> `docker_*` (daemon) is distinct from `docker_search`/`docker_image`/`docker_tags`
> (keyless Docker Hub lookups, above), which are unaffected by `[docker]`.

## Kubernetes cluster

A **cluster-control** capability: lodestone talks to the Kubernetes API server
directly via [kube-rs](https://kube.rs), reading your kubeconfig (default location,
`$KUBECONFIG`, or a configured path/context) or in-cluster credentials — no
`kubectl`. Code: [`src/skills/kubernetes.rs`](../src/skills/kubernetes.rs). Gated by `[kubernetes]` (see
[`config/09-kubernetes.toml`](../config/09-kubernetes.toml)); `enabled` on by
default. `k8s_delete` is always exposed but requires a per-call confirmation token
(`confirm`/`trust` — see [tools.md](tools.md#confirming-destructive-actions));
`allow_destructive` pre-authorizes it. `kind` accepts kubectl-style names (`pods`,
`deploy`, `svc`, `nodes`, …) resolved via API discovery.

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `k8s_contexts` | — | read | List kubeconfig contexts + current (no cluster contact). |
| `k8s_get` | `kind`, `name?`, `namespace?` | read | Get one object (JSON) or list a kind. |
| `k8s_describe` | `kind`, `name`, `namespace?` | read | Full JSON of one named object. |
| `k8s_logs` | `pod`, `namespace?`, `container?`, `tail?` | read | A pod's logs. |
| `k8s_apply` | `manifest` | write | Server-side apply a kubefile (multi-doc YAML). |
| `k8s_scale` | `kind`, `name`, `replicas`, `namespace?` | write | Scale a workload. |
| `k8s_delete` | `kind`, `name`, `namespace?`, `confirm?`, `trust?` | **destructive** | Delete a resource (confirm first). |

> Helm release *mutation* (install/upgrade/uninstall) would mean reimplementing
> Helm, so it's out of scope for the direct-API approach; `docs_helm` and
> `artifacthub_search` cover Helm discovery instead.

## Documentation search

The framework-docs family also covers tooling docs: `docs_docker`
(docs.docker.com), `docs_kubernetes` (kubernetes.io), and `docs_helm` (helm.sh)
are on by default and join `docs_search`. See
[providers/frameworks.md](providers/frameworks.md).
