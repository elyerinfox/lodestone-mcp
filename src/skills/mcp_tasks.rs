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
