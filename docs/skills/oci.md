# Container images — `docker_search` / `docker_image` / `docker_tags` / `oci_tags` / `oci_manifest`

|  |  |
| --- | --- |
| **Module** | [`src/skills/oci.rs`](../../src/skills/oci.rs) |
| **Tools** | `docker_search`, `docker_image`, `docker_tags`, `oci_tags`, `oci_manifest` |
| **Network** | keyless Docker Hub JSON API + OCI Distribution Spec (anonymous bearer-token pull) |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Read-only lookups for container images, with no account or key. The three
`docker_*` tools query Docker Hub's `hub.docker.com` JSON API (search, repository
details, tag listings). The two `oci_*` tools inspect **any** registry implementing
the OCI Distribution Spec — Docker Hub, GHCR (`ghcr.io`), Quay (`quay.io`), or a
self-hosted registry — using the spec's anonymous bearer-token flow (a public pull
gets a `401` with a `WWW-Authenticate: Bearer realm=…` challenge, a token is fetched
from that realm with no credentials, then the request is retried). These are data
lookups, distinct from the local Docker-daemon control skill in
[`src/skills/docker.rs`](../../src/skills/docker.rs). All results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `docker_search` | `query`, `max_results?` | Search Docker Hub for images: name, official/verified status, stars, pull count, short description. `max_results` defaults to 10, capped at 25. |
| `docker_image` | `image` | One Docker Hub repository's details: description, stars, pulls, last-updated date, and long description. `image` is `nginx` (official → `library/nginx`), `library/nginx`, or `org/image`; an optional `:tag` is ignored (it describes the repo). |
| `docker_tags` | `image`, `max_results?` | List a Docker Hub image's tags (newest first) with compressed size, last-pushed date, and architectures. `max_results` defaults to 15, capped at 50. |
| `oci_tags` | `reference`, `max_results?` | List tags for an image on any OCI registry via `/tags/list`. `reference` is `nginx`, `ghcr.io/owner/image`, `quay.io/ns/repo`, `localhost:5000/team/app`, etc. `max_results` defaults to 30, capped at 200. |
| `oci_manifest` | `reference` | Inspect one image's manifest on any OCI registry: for a multi-arch index, the platforms (`os/arch[/variant]`); for a single image, the layer count, total compressed size, and config digest, plus the content digest. `reference` may carry `:tag` (default `latest`) or `@sha256:<digest>`. |

`reference`/`image` parsing: the registry host is taken from the first path
segment when it looks like a domain or has a port (e.g. `localhost:5000`);
otherwise it resolves to Docker Hub, and a bare official name like `nginx` becomes
`library/nginx`. The `docker_*` tools reject non-Docker-Hub references and point
you at the `oci_*` equivalents.

## Configuration & gating
No configuration. All five tools are on by default with no tunables; disable any
of them through `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)).
These keyless lookups are unaffected by the `[docker]` section, which gates the
separate local-daemon control tools.

## Example uses
- **Pick an image, then verify platform support** — `docker_search` for "postgres" to find candidates, `docker_tags` with `image="postgres"` to see available tags, then `oci_manifest` with `reference="postgres:16"` to confirm it is multi-arch (e.g. includes `linux/arm64`).
- **Compare image size across tags** — `docker_tags` with `image="grafana/grafana"` to read compressed sizes per tag.
- **Inspect a GHCR image** — `oci_tags` with `reference="ghcr.io/owner/image"` to list tags, then `oci_manifest` with `reference="ghcr.io/owner/image:latest"` for its platforms or layer total.
- **Check whether an image is official** — `docker_image` with `image="nginx"` for stars, pulls, and the long description.

## See also
- [containers.md](../containers.md) — overview of all container/cloud-native tools, including the local Docker daemon and Kubernetes cluster control.
- [tools.md](../tools.md) — full tool reference.
