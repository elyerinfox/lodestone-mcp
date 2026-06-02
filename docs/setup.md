# Connecting lodestone to your MCP host

Lodestone speaks **MCP over Streamable HTTP** at `/mcp`. Any compliant
MCP host can connect — the only setup is pointing the host at
`http://127.0.0.1:8000/mcp` (or wherever you've bound it).

For a generic / unknown host see **[the protocol shape](#protocol-shape)**
at the bottom of this page. The sections below cover the specific configs
for popular MCP hosts.

## Prerequisites

You need lodestone running and reachable. Pick **one** of the three
paths below — all three end up with the MCP server listening on
`http://127.0.0.1:8000/mcp`. After that the host-specific snippets
later in this document apply identically.

After whichever path you pick:

- `GET http://127.0.0.1:8000/health` returns `ok`.
- If you've set `auth_token` in `lodestone.toml`, the host must send
  `Authorization: Bearer <token>`. See
  **[docs/configuration.md → Auth](configuration.md#auth)**.
- A copy of the canonical client config shape lives in
  [`mcp.example.json`](../mcp.example.json) at the repo root.

### Path 1 — Docker (the simple one)

The easiest way. Requires only Docker (any modern version) and
`docker compose`. No Rust toolchain, no Node, no system deps.

```sh
docker compose up --build
# → MCP server   http://localhost:8000   (lodestone-mcp,    from ./Dockerfile)
# → Dashboard    http://localhost:8001   (lodestone-dashboard, from frontend/Dockerfile)
```

Skip the dashboard with `docker compose up --build lodestone` — the
MCP server runs standalone.

> ### Docker mode — host-dependency limitations
>
> Running lodestone inside a container gives you simplicity at the cost
> of access to several host-side things the local-system skills need.
> The following skill families are **unavailable or limited** in the
> default Docker setup unless you explicitly opt in to additional
> mounts / device passthrough / network modes:
>
> | Family | Why it fails | Workaround if you need it |
> | --- | --- | --- |
> | `docker_*` (control your local Docker daemon) | The container can't talk to the host Docker socket by default. | Mount `/var/run/docker.sock` into the container, or set `DOCKER_HOST` to a reachable daemon. **Note the security trade-off** — exposing the socket gives the container root-equivalent control over the host. |
> | `kubernetes_*` | No kubeconfig inside the container. | Mount `~/.kube/config` read-only and set `KUBECONFIG`. |
> | `filesystem_*` | The container only sees its own filesystem; `[filesystem].roots` you configure point at *container* paths, not host paths. | Bind-mount the host directories you want lodestone to reach, then set `roots` to the bind targets. |
> | `shell_*` | Runs inside the container's userspace, not the host's. Programs installed on the host are not on the container's `$PATH`. | Build a custom image that adds the binaries you need (`apt install`...), or use `--network host` + a host-installed `mcp-remote`-style bridge. |
> | `git_*` | Same story as `filesystem_*` — needs the repos bind-mounted in. | Bind-mount the working trees. |
> | `serial_*`, `printer_*`, `sdr_*` | Hardware device passthrough is OS-specific (`--device /dev/ttyUSB0`, USB passthrough on macOS/Windows is platform-specific). | Add the matching `--device` / `--cap-add` lines or use `--privileged` (not recommended). |
> | `python_*`, `ffmpeg_*`, `packages_*` | The image is minimal — these binaries aren't included by default. | Build a custom image extending lodestone with the binaries you need; or use Path 2/3 below. |
> | `mqtt_*`, `meshtastic_*` | Outbound MQTT works fine from inside the container; inbound (subscribing to a host-side broker) needs the host's broker reachable on the network. | Use a TCP broker on a routable address, or run host networking. |
> | `sysinfo_*` GPU probes | NVML / DRM sysfs / vendor tools live on the host. | Add `--gpus all` and the matching driver passthrough. NVIDIA Container Toolkit is the supported route. |
> | `printer_*` | CUPS / Windows print services live on the host. | Host networking + the host's CUPS server reachable, or bind-mount the printer device. |
>
> **All of the keyless web / retrieval / chemistry / biology / nuclear
> / radiology / machinist / CNC / math / geodesy / RF / radar / DSP /
> info-theory / crypto-math / forecast / format-converter / chart /
> open-data feeds work the same in Docker as on bare metal.** It's the
> "talk to the host's daemons and devices" half of the toolkit that
> needs the workarounds above.
>
> If your use case is heavy on local-system control, prefer Path 2 or
> Path 3 (native build / Cargo install) and skip the Docker indirection.

### Path 2 — Build from source

For everything Docker can't easily expose, or when you want the fastest
possible runtime. Requires a recent Rust toolchain
([rust-lang.org/tools/install](https://www.rust-lang.org/tools/install)).
Node is **not** needed for the MCP server (the dashboard is a separate
service; see [docs/building.md](building.md) if you also want it).

```sh
# Clone the repo.
git clone https://github.com/elyerinfox/lodestone-mcp.git
cd lodestone-mcp

# Debug build for iteration; release for production.
cargo run --bin lodestone-mcp                     # debug profile
cargo run --release --bin lodestone-mcp           # ~3× faster startup, faster math

# Or build first and run the binary directly:
cargo build --release --bin lodestone-mcp
./target/release/lodestone-mcp
```

`--bin lodestone-mcp` is required because the crate ships two binaries
(the MCP server and the optional `lodestone-galaxy` rendezvous broker;
see
[docs/constellation.md](constellation.md#galaxy--linking-constellations)).

Optional runtime dependencies are detected at startup and gate the
matching skill families:

- **Chrome / Chromium** — only needed when you actually use
  `render=true`, the `google` engine, or the `browser_*` family.
  Capability probe fires once at startup; if no Chrome is on `$PATH`,
  those tools return a clean "unavailable" error rather than crash.
- **Docker daemon** — gates `docker_*`.
- **kubeconfig** — gates `kubernetes_*`.
- **`ffmpeg` on `$PATH`** — gates `ffmpeg_*`.
- **`python3` on `$PATH`** — gates `python_*`.
- **`git` on `$PATH`** — gates `git_*`.
- **A serial / printer / SDR device** — gates the matching hardware
  family.
- **A configured MQTT broker** — gates `mqtt_*` and `meshtastic_*`.

The full per-family build / dev workflow (including the dashboard
HMR loop, the Nuxt build, image build, and the `make` shortcuts) lives
in **[docs/building.md](building.md)**.

### Path 3 — `cargo install`

For when you don't want to clone the repo. Requires the same Rust
toolchain as Path 2.

```sh
cargo install --git https://github.com/elyerinfox/lodestone-mcp \
              --bin lodestone-mcp

# `lodestone-mcp` is now on your $PATH:
lodestone-mcp
```

This puts a release-mode binary at `$CARGO_HOME/bin/lodestone-mcp`. To
upgrade, re-run with `--force`. To uninstall, `cargo uninstall
lodestone-mcp`. No separate config file is shipped by this path — set
`LODESTONE_*` env vars or drop a `lodestone.toml` in the CWD. The
shipped `config/` baseline doesn't come along with `cargo install`;
clone the repo if you want it (or grab the files individually from
GitHub).

## LM Studio

LM Studio reads `mcp.json` from the platform's LM Studio config dir.

- **Windows**: `%USERPROFILE%\.lmstudio\mcp.json`
- **macOS / Linux**: `~/.lmstudio/mcp.json`

```json
{
  "mcpServers": {
    "lodestone": { "url": "http://127.0.0.1:8000/mcp" }
  }
}
```

If `auth_token` is set, add a `headers` block:

```json
{
  "mcpServers": {
    "lodestone": {
      "url": "http://127.0.0.1:8000/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

## Claude Code (CLI)

Claude Code registers MCP servers from the shell. One-time setup:

```sh
claude mcp add lodestone http://127.0.0.1:8000/mcp --transport http
```

With auth:

```sh
claude mcp add lodestone http://127.0.0.1:8000/mcp \
  --transport http \
  --header "Authorization: Bearer <token>"
```

Verify with `claude mcp list`.

## Claude Desktop

Claude Desktop's MCP wiring is **stdio-first** — it does not yet support
Streamable HTTP natively. Bridge HTTP to stdio with the community
`mcp-remote` adapter.

Config file:

- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`
- **macOS**:   `~/Library/Application Support/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "lodestone": {
      "command": "npx",
      "args": ["mcp-remote", "http://127.0.0.1:8000/mcp"]
    }
  }
}
```

`mcp-remote` is shipped on npm (`npx mcp-remote ...`) so no extra
install step is needed. For an auth token, pass `--header
"Authorization: Bearer <token>"` to `mcp-remote`.

## Continue (VS Code / JetBrains)

Continue reads `~/.continue/config.json`. Add an `mcpServers` block:

```json
{
  "mcpServers": {
    "lodestone": { "url": "http://127.0.0.1:8000/mcp" }
  }
}
```

The block lives at the top level of the Continue config; restart the
IDE after saving.

## Cline (VS Code)

Cline reads `cline_mcp_settings.json`. Open the Cline panel →
**MCP Servers** → **Edit MCP Settings**, and add the same shape:

```json
{
  "mcpServers": {
    "lodestone": { "url": "http://127.0.0.1:8000/mcp" }
  }
}
```

## Cursor

Cursor reads `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "lodestone": { "url": "http://127.0.0.1:8000/mcp" }
  }
}
```

Restart Cursor after editing.

## Codename Goose

Goose reads `~/.config/goose/config.yaml`:

```yaml
extensions:
  lodestone:
    type: http
    uri: http://127.0.0.1:8000/mcp
```

## Zed

Zed's `settings.json` (per-project or global):

```json
{
  "context_servers": {
    "lodestone": { "url": "http://127.0.0.1:8000/mcp" }
  }
}
```

## Any other MCP-capable host

Point your host at the Streamable-HTTP endpoint:

```
http://127.0.0.1:8000/mcp
```

Hosts that only support **stdio MCP** can bridge via the community
`mcp-remote` shim (the same pattern Claude Desktop uses above). Install
once and have your host launch `npx mcp-remote http://127.0.0.1:8000/mcp`
as the server command.

## Protocol shape

For reference — these are the bits any host needs to talk to a lodestone
instance:

| Field | Value |
| --- | --- |
| Transport | Streamable HTTP (MCP 2025-03-26 spec) |
| Endpoint | `http://127.0.0.1:8000/mcp` (configurable via `bind`) |
| Auth | Optional `Authorization: Bearer <auth_token>`. Off by default. |
| Health probe | `GET /health` returns `ok` (always public). |
| Dashboard WS | `GET ws://127.0.0.1:8000/ws/status?token=<network.token>` — separate trust domain. |

If your host can't reach `localhost`, expose the bind address (e.g.
`bind = "0.0.0.0:8000"`) and point the host at the LAN address instead.
For production behind TLS, terminate at your reverse proxy and point
the host at the HTTPS URL.

## Troubleshooting

- **`tools/list` returns nothing.** Check that the dispatch wrapper's
  capability cache isn't gating every tool — `GET /api/settings` returns
  the runtime view. Most often this is `auth_token` mismatched between
  the host header and `lodestone.toml`.
- **Tools time out.** Default per-call timeout is 30 s; long-running
  fetches should use the `background: true` global argument (returns a
  `task_id` immediately and pushes progress notifications).
- **The host says "no such tool"** after adding a new skill — restart
  the host so it re-issues `tools/list`. MCP hosts do not poll the list
  on their own.
- **Hosts that only support stdio** — install
  [`mcp-remote`](https://www.npmjs.com/package/mcp-remote) (`npm install
  -g mcp-remote`) and bridge.
