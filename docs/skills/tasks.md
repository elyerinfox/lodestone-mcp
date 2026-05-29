# Background tasks — `task_run`, `task_list`, `task_status`, `task_result`, `task_cancel`

|  |  |
| --- | --- |
| **Module** | [`src/skills/tasks.rs`](../../src/skills/tasks.rs) |
| **Tools** | `task_run`, `task_list`, `task_status`, `task_result`, `task_cancel` |
| **Network** | n/a (orchestrates other work) |
| **Default** | **off** — gated by `[tasks]` |
| **Config** | `[tasks]` in [`config/01-tools.toml`](../../config/01-tools.toml) |

## What it does
Runs long work **in the background** so the model isn't blocked on it, then lets the
model **poll** for the result. Delivery is a model-polled results buffer — no
server-initiated notifications — so it works on **any** MCP client, including ones
(like LM Studio) that don't support server push. The job table is bounded and
results are evicted oldest-first, so a runaway fan-out can't exhaust the host.

This lets the model parallelize itself: kick off several `task_run` searches at once,
keep reasoning, then collect results with `task_result`.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `task_run` | `op?` (=`search`), `kind`, `query`, `max_results?` | Start a background job; returns a task id immediately. |
| `task_list` | — | List tasks (id, status, label, age), newest first. |
| `task_status` | `id` | One task's status: running / done / failed / cancelled. |
| `task_result` | `id` | The result if done (else says still-running / the error). |
| `task_cancel` | `id` | Cancel a running task (no-op if already finished). |

Currently the backgroundable operation is **search**: `task_run { kind: "web"|"code"
|"docs"|"qa", query: "…" }`. It runs from owned handles (the search registry + HTTP
client). The registry and the four management tools are the foundation; other long
tools can be wired to background later.

## Example flow
1. `task_run { kind: "web", query: "rust async runtimes" }` → `task-1`.
2. `task_run { kind: "docs", query: "tokio select" }` → `task-2`.
3. …keep working…
4. `task_result { id: "task-1" }` and `task_result { id: "task-2" }` to collect.

## See also
[tools.md](../tools.md)
