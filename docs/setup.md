# Connecting lodestone to your MCP host

Lodestone speaks **MCP over Streamable HTTP** at `/mcp`. Any compliant
MCP host can connect — the only setup is pointing the host at
`http://127.0.0.1:8000/mcp` (or wherever you've bound it).

For a generic / unknown host see **[the protocol shape](#protocol-shape)**
at the bottom of this page. The sections below cover the specific configs
for popular MCP hosts.

## Prerequisites

1. The server is running. From the repo:

   ```sh
   cargo run --bin lodestone-mcp
   ```

   (`--bin` is required because the crate ships two binaries —
   `lodestone-mcp` and `lodestone-galaxy`; see
   [docs/constellation.md](constellation.md#galaxy--linking-constellations).)

2. `GET http://127.0.0.1:8000/health` returns `ok`.

3. If you've set `auth_token` in `lodestone.toml`, the host must send
   `Authorization: Bearer <token>`. See
   **[docs/configuration.md → Auth](configuration.md#auth)**.

A copy of the canonical client config shape lives in
[`mcp.example.json`](../mcp.example.json) at the repo root.

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
