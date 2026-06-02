//! MCP Tasks primitive — `tasks_list` / `tasks_get` / `tasks_result` /
//! `tasks_cancel` (the 2025-11-25 spec's `tasks/list`, `tasks/get`,
//! `tasks/result`, `tasks/cancel` exposed as plain MCP tools so every
//! client can drive them today, regardless of whether the client has
//! taught its model the native method shape).
//!
//! Thin wrappers over [`crate::tasks::TaskRuntime`]. The actual lifecycle
//! and notifications live there.
//!
//! These are deliberately distinct from the legacy `task_*` (singular)
//! skill in [`crate::skills::tasks`], which is a polling-only background
//! search buffer. New skills wanting MCP-style async should spawn into
//! [`crate::tasks::TaskRuntime`] and let the model drive them through
//! these tools (and the corresponding `notifications/progress` +
//! `notifications/tasks/status` pushes).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TaskIdArgs {
    /// Task id returned by a prior async-launched tool (e.g. `mqtt_listen`).
    task_id: String,
}

pub struct TasksList;
impl Skill for TasksList {
    fn name(&self) -> &'static str {
        "tasks_list"
    }
    fn description(&self) -> &'static str {
        "List currently-tracked async tasks (newest first). Each row has the \
        task id, kind, label, status (`working` / `completed` / `failed` / \
        `cancelled`), last progress, and how long ago it was last updated. \
        Use `tasks_get` for one task's metadata or `tasks_result` for a \
        finished one's body. Mirrors the MCP `tasks/list` method."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let rows = ctx.server.task_runtime.list().await;
            if rows.is_empty() {
                return Ok(text_result("No tracked tasks."));
            }
            let now = now_secs();
            let mut out = format!("{} task(s):", rows.len());
            for r in rows {
                out.push_str(&format!(
                    "\n  {} [{}] {} ({}, updated {}s ago)",
                    r.task_id,
                    r.status.as_str(),
                    r.label,
                    r.kind,
                    now.saturating_sub(r.updated_unix)
                ));
                if let Some(p) = r.progress {
                    match r.total {
                        Some(t) => out.push_str(&format!("  {p}/{t}")),
                        None => out.push_str(&format!("  progress={p}")),
                    }
                }
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "List every tracked task",
            args: r#"{}"#,
            note: Some("Newest first; each row has id, status, label, kind, age, last progress."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "See what background work is in flight before launching more.",
            "Find a forgotten `task_id` to inspect or cancel via the other `tasks_*` tools.",
        ]
    }
}

pub struct TasksGet;
impl Skill for TasksGet {
    fn name(&self) -> &'static str {
        "tasks_get"
    }
    fn description(&self) -> &'static str {
        "Fetch one task's metadata: id, kind, label, status, created/updated \
        timestamps, last progress + message. For the actual body of a finished \
        task, use `tasks_result`. Mirrors the MCP `tasks/get` method."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TaskIdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<TaskIdArgs>()?;
            let Some(info) = server.task_runtime.get(&args.task_id).await else {
                return Err(invalid(format!("no task with id {}", args.task_id)));
            };
            let body =
                serde_json::to_string_pretty(&info).map_err(|e| crate::internal(e.into()))?;
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Get one task's metadata",
                args: r#"{"task_id": "task-3"}"#,
                note: Some("Returns JSON with id, kind, label, status, timestamps, last progress. Use `tasks_result` for the body."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check whether a specific task is still working, completed, or failed.",
            "Read a task's label and last progress message without pulling its body.",
        ]
    }
}

pub struct TasksResult;
impl Skill for TasksResult {
    fn name(&self) -> &'static str {
        "tasks_result"
    }
    fn description(&self) -> &'static str {
        "Fetch a task's terminal result (or its progress log so far while still \
        running). Returns `{task_id, status, result?, error?, progress_log[]}`. \
        Progress log entries replay what the task pushed via \
        `notifications/progress`. Mirrors the MCP `tasks/result` method."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TaskIdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<TaskIdArgs>()?;
            let Some(r) = server.task_runtime.result(&args.task_id).await else {
                return Err(invalid(format!("no task with id {}", args.task_id)));
            };
            let body = serde_json::to_string_pretty(&r).map_err(|e| crate::internal(e.into()))?;
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Fetch a finished task's body",
            args: r#"{"task_id": "task-3"}"#,
            note: Some("Returns `{task_id, status, result?, error?, progress_log[]}`."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pick up the output of a background `search_async` / listener task.",
            "Read the replayed progress log of a task that's still working.",
        ]
    }
}

pub struct TasksCancel;
impl Skill for TasksCancel {
    fn name(&self) -> &'static str {
        "tasks_cancel"
    }
    fn description(&self) -> &'static str {
        "Cancel a running task. Fires the task's cancellation token (so the \
        body wakes and unwinds) and pushes `notifications/tasks/status` with \
        `status: \"cancelled\"`. No-op if the task is already terminal. \
        Mirrors the MCP `tasks/cancel` method."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TaskIdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<TaskIdArgs>()?;
            let did = server.task_runtime.cancel(&args.task_id).await;
            Ok(text_result(if did {
                format!("Cancelled {}.", args.task_id)
            } else {
                format!(
                    "Task {} is not running (already terminal, or no such task).",
                    args.task_id
                )
            }))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Cancel a running task",
            args: r#"{"task_id": "task-3"}"#,
            note: Some("No-op if the task is already terminal."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Stop a background task that's no longer needed (search / listener).",
            "Free up runtime slots when a fan-out turned up enough results early.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(TasksList),
        Box::new(TasksGet),
        Box::new(TasksResult),
        Box::new(TasksCancel),
    ]
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
