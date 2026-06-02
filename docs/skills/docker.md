# Local Docker daemon — `docker_ps` / `docker_run` / `docker_exec`

|  |  |
| --- | --- |
| **Module** | [`src/skills/docker.rs`](../../src/skills/docker.rs) |
| **Tools** | `docker_ps`, `docker_images`, `docker_inspect`, `docker_logs`, `docker_info`, `docker_pull`, `docker_run`, `docker_start`, `docker_build`, `docker_stop`, `docker_remove`, `docker_exec`, `docker_rmi` |
| **Network** | local Docker daemon (Engine API over the platform socket) |
| **Default** | on — gated by `[docker]` |
| **Config** | `[docker]` in [`config/08-docker.toml`](../../config/08-docker.toml) |

## What it does
A **local-system** capability, distinct from the keyless Docker Hub lookups
(`docker_search`/`docker_image`/`docker_tags`). lodestone talks to your Docker
daemon directly via the Engine API over the platform socket — the Windows named
pipe `\\.\pipe\docker_engine`, the unix socket, or whatever `DOCKER_HOST` points
at (via [bollard](https://crates.io/crates/bollard)) — and never invokes the
`docker` CLI. Each action is its own tool so an MCP host can grant permission at
per-action granularity.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `docker_ps` | `all?` | read | List containers — running only, or all (incl. stopped) with `all=true`. |
| `docker_images` | — | read | List images on the daemon (id, tags, size). |
| `docker_inspect` | `container` | read | Full JSON for one container (config, state, mounts, networks). |
| `docker_logs` | `container`, `tail?` | read | A container's stdout+stderr (last `tail` lines; default 200, capped 2000). |
| `docker_info` | — | read | Daemon version, API version, os/arch, container/image counts. |
| `docker_pull` | `image`, `confirm?`, `trust?` | destructive | Pull an image, e.g. `nginx:1.27` or `ghcr.io/owner/image:tag` (confirm first — network egress + writes to the local image store). |
| `docker_run` | `image`, `name?`, `command?`, `confirm?`, `trust?` | destructive | Create + start a container; `command` is split on whitespace (confirm first — effectively arbitrary code execution under the image's entrypoint). |
| `docker_start` | `container`, `confirm?`, `trust?` | destructive | Start an existing (stopped) container (confirm first — resumes a process that may bind ports, mount volumes, or execute its entrypoint). |
| `docker_build` | `context`, `tag`, `dockerfile?`, `confirm?`, `trust?` | destructive | Build an image from a context dir (tarred + sent to the daemon); `dockerfile` defaults to `Dockerfile` (confirm first — every Dockerfile RUN step is arbitrary code execution under the daemon). |
| `docker_stop` | `container`, `confirm?`, `trust?` | destructive | Stop a running container (confirm first). |
| `docker_remove` | `container`, `force?`, `confirm?`, `trust?` | destructive | Remove a container; `force` kills a running one (confirm first). |
| `docker_exec` | `container`, `command`, `confirm?`, `trust?` | destructive | Run a command inside a running container (parsed like a shell line, executed directly — no host shell); returns combined stdout/stderr (confirm first). |
| `docker_rmi` | `image`, `force?`, `confirm?`, `trust?` | destructive | Remove an image; `force` removes a tagged/in-use image (confirm first). |

## Configuration & gating
The `[docker]` section in
[`config/08-docker.toml`](../../config/08-docker.toml) has two switches:

- `enabled` (default `true`, env `LODESTONE_DOCKER_ENABLED`) — exposes the whole
  family. When off, all `docker_*` daemon tools disappear (gating lives in
  `main.rs::effective_disabled`). The keyless Docker Hub tools are unaffected.
- `allow_destructive` (default `false`, env
  `LODESTONE_DOCKER_ALLOW_DESTRUCTIVE`) — **pre-authorizes** the destructive
  tools, skipping the confirmation prompt below.

**Confirmation guard.** `docker_pull`, `docker_run`, `docker_start`, `docker_stop`,
`docker_remove`, `docker_exec`, `docker_rmi`, and `docker_build` are always
exposed but route through the confirmation
[`guard`](../../src/skills/guard.rs) (golden rule 8). Bindings are
**per-target** (e.g. `docker_pull:<image>`, `docker_run:<image>|<name>|<command>`,
`docker_start:<container>`, `docker_build:<context>|<dockerfile>|<tag>`), so
`trust=true` only whitelists that specific target — pulling `nginx:1.27` does not
authorize a later `docker_pull alpine`. The **first** call performs
nothing — it returns a one-time `confirm` token describing exactly what will
happen. Call the tool again with `confirm=<token>` to actually run it, or
`confirm=<token>` plus `trust=true` to also stop being asked for that tool for
the rest of the session. Tokens are single-use and expire after 5 minutes. This
works on any MCP client (no elicitation support required). Setting
`allow_destructive` pre-authorizes the action and skips the prompt entirely.

## Example uses
- **Triage a failing container** — `docker_ps` (find the name/id) → `docker_logs`
  → `docker_inspect` to see why it's unhealthy.
- **Bring up a service** — `docker_pull nginx:alpine` → `docker_run` (with a
  `name`) → `docker_ps` to confirm it's running.
- **Build and verify an image** — `docker_build` (context + tag) →
  `docker_images` to confirm the tag, → `docker_run` it.
- **Tear down** — `docker_stop` (returns a token, call again with `confirm`) →
  `docker_remove` → `docker_rmi` to reclaim the image (each confirms separately).

## See also
[containers.md](../containers.md), [golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
