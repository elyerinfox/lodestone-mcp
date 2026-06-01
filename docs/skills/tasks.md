# Async search — `search_async`

|  |  |
| --- | --- |
| **Module** | [`src/skills/tasks.rs`](../../src/skills/tasks.rs) |
| **Tools** | `search_async` |
| **Default** | **off** — gated by `[tasks].enabled` |
| **Config** | `[tasks]` in [`config/01-tools.toml`](../../config/01-tools.toml) |

## What it does
Launches a search (`web`/`code`/`docs`/`qa`) as a **background task** in the
shared [`TaskRuntime`](../../src/tasks.rs) and returns a `task_id` immediately.
Lets the model fan out several searches at once instead of serializing on each.

Management (list, poll, fetch result, cancel) goes through the MCP-spec
`tasks_*` tools (`tasks_list`, `tasks_get`, `tasks_result`, `tasks_cancel`) —
they read the same runtime, so the same `task_id` works from either surface.
The runtime also backs `mqtt_listen` and `meshtastic_listen`; every
backgrounded job in the codebase shows up in one inspection surface.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `search_async` | `kind` (`web`/`code`/`docs`/`qa`), `query`, `max_results?` (1–25) | Start a background search; returns `task_id` immediately. |

## Notifications
If the caller's request includes `_meta.progressToken`, the runtime emits:
- `notifications/progress` once at "searching…" and once at "N hits via <engine>".
- `notifications/tasks/status` on completion (full task object with `status: "completed"` + the formatted hits).

Clients without notification support still get full functionality via
`tasks_result` polling.

## Example flow
1. `search_async { kind: "web", query: "rust async runtimes" }` → `task-1`.
2. `search_async { kind: "docs", query: "tokio select" }` → `task-2`.
3. (Reason about something else while both run.)
4. `tasks_result { task_id: "task-1" }` and `tasks_result { task_id: "task-2" }` to collect.

## History
This module previously hosted `task_run` / `task_list` / `task_status` /
`task_result` / `task_cancel` — a self-contained polling-only registry.
Those tools were collapsed into the shared `TaskRuntime`: `task_run` was
renamed `search_async` (its only real job was launching a search); the
four management tools were dropped because the MCP-spec `tasks_*` tools
already provide that surface against the same registry.

## See also
[tools.md](../tools.md), [mqtt.md](mqtt.md), [meshtastic.md](meshtastic.md)
