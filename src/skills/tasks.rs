//! Background-tasks skill — run long work without blocking the tool call, then poll
//! for the result. **Off by default** (`[tasks].enabled`).
//!
//! Delivery is model-polled (a results buffer), which works on **any** MCP client
//! including ones without server-initiated notifications (e.g. LM Studio): `task_run`
//! returns a task id immediately, and the model later calls `task_result`. The job
//! table is bounded and results are kept until evicted, so a runaway fan-out can't
//! exhaust the host.
//!
//! Currently the backgroundable operation is **search** (`web`/`code`/`docs`/`qa`),
//! the main long/aggregate call — it runs from owned handles (`Arc<Registry>` + the
//! HTTP client). The registry + the four management tools are the foundation; other
//! tools can be wired to background later (see TODO).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use tokio::task::AbortHandle;

use crate::provider::{ProviderKind, SearchQuery, SearchResult};
use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::{invalid, text_result};

/// Tool names (gated by `[tasks].enabled` in `disabled_by_config`).
pub const TOOL_NAMES: &[&str] = &[
    "task_run",
    "task_list",
    "task_status",
    "task_result",
    "task_cancel",
];

/// Most jobs retained in the table; creating past this evicts the oldest finished ones.
const MAX_JOBS: usize = 64;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    fn as_str(&self) -> &'static str {
        match self {
            Status::Running => "running",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        }
    }
}

struct Job {
    id: String,
    label: String,
    status: Status,
    result: Option<String>,
    created: u64,
    finished: Option<u64>,
    abort: Option<AbortHandle>,
}

struct Inner {
    jobs: HashMap<String, Job>,
    seq: AtomicU64,
}

/// A shared, bounded background-job registry (cloneable; all clones share state).
#[derive(Clone)]
pub(crate) struct Tasks(Arc<Mutex<Inner>>);

impl Tasks {
    pub(crate) fn new() -> Self {
        Tasks(Arc::new(Mutex::new(Inner {
            jobs: HashMap::new(),
            seq: AtomicU64::new(1),
        })))
    }

    /// Register a new running job and return its id. Evicts the oldest finished jobs
    /// when over capacity.
    fn create(&self, label: String) -> String {
        let mut inner = self.0.lock().unwrap();
        let n = inner.seq.fetch_add(1, Ordering::Relaxed);
        let id = format!("task-{n}");
        if inner.jobs.len() >= MAX_JOBS {
            // Evict oldest finished jobs first.
            let mut finished: Vec<(String, u64)> = inner
                .jobs
                .values()
                .filter(|j| j.status != Status::Running)
                .map(|j| (j.id.clone(), j.finished.unwrap_or(j.created)))
                .collect();
            finished.sort_by_key(|(_, t)| *t);
            for (vid, _) in finished.into_iter().take(inner.jobs.len() + 1 - MAX_JOBS) {
                inner.jobs.remove(&vid);
            }
        }
        inner.jobs.insert(
            id.clone(),
            Job {
                id: id.clone(),
                label,
                status: Status::Running,
                result: None,
                created: now_secs(),
                finished: None,
                abort: None,
            },
        );
        id
    }

    fn attach_abort(&self, id: &str, abort: AbortHandle) {
        let mut inner = self.0.lock().unwrap();
        if let Some(j) = inner.jobs.get_mut(id) {
            if j.status == Status::Running {
                j.abort = Some(abort);
            }
        }
    }

    /// Mark a job done (`Ok`) or failed (`Err`) with its output/message.
    fn finish(&self, id: &str, outcome: Result<String, String>) {
        let mut inner = self.0.lock().unwrap();
        if let Some(j) = inner.jobs.get_mut(id) {
            if j.status != Status::Running {
                return; // already cancelled/finished
            }
            j.finished = Some(now_secs());
            j.abort = None;
            match outcome {
                Ok(text) => {
                    j.status = Status::Done;
                    j.result = Some(text);
                }
                Err(e) => {
                    j.status = Status::Failed;
                    j.result = Some(e);
                }
            }
        }
    }

    fn cancel(&self, id: &str) -> Option<bool> {
        let mut inner = self.0.lock().unwrap();
        let j = inner.jobs.get_mut(id)?;
        if j.status != Status::Running {
            return Some(false); // nothing to cancel
        }
        if let Some(a) = j.abort.take() {
            a.abort();
        }
        j.status = Status::Cancelled;
        j.finished = Some(now_secs());
        Some(true)
    }

    fn list(&self) -> String {
        let inner = self.0.lock().unwrap();
        if inner.jobs.is_empty() {
            return "No background tasks.".to_string();
        }
        let now = now_secs();
        let mut rows: Vec<(u64, String)> = inner
            .jobs
            .values()
            .map(|j| {
                let age = now.saturating_sub(j.created);
                (
                    j.created,
                    format!(
                        "  {} [{}] {} ({}s ago)",
                        j.id,
                        j.status.as_str(),
                        j.label,
                        age
                    ),
                )
            })
            .collect();
        rows.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
        let body: Vec<String> = rows.into_iter().map(|(_, s)| s).collect();
        format!("Background tasks ({}):\n{}", body.len(), body.join("\n"))
    }

    fn status(&self, id: &str) -> Option<String> {
        let inner = self.0.lock().unwrap();
        let j = inner.jobs.get(id)?;
        Some(format!("{} [{}] {}", j.id, j.status.as_str(), j.label))
    }

    fn result(&self, id: &str) -> Option<(Status, String, String)> {
        let inner = self.0.lock().unwrap();
        let j = inner.jobs.get(id)?;
        Some((
            j.status,
            j.label.clone(),
            j.result.clone().unwrap_or_default(),
        ))
    }
}

/// Compact rendering of search hits for a background result buffer.
fn format_hits(hits: &[SearchResult], engine: &str) -> String {
    let mut lines = vec![format!("{} result(s) via {engine}:", hits.len())];
    for (i, h) in hits.iter().enumerate() {
        lines.push(format!("\n{}. {}", i + 1, h.title));
        if !h.url.is_empty() {
            lines.push(format!("   {}", h.url));
        }
        if !h.snippet.is_empty() {
            let s: String = h.snippet.chars().take(200).collect();
            lines.push(format!("   {s}"));
        }
    }
    lines.join("\n")
}

fn parse_kind(s: &str) -> Option<ProviderKind> {
    match s.trim().to_ascii_lowercase().as_str() {
        "web" => Some(ProviderKind::Web),
        "code" => Some(ProviderKind::Code),
        "docs" => Some(ProviderKind::Docs),
        "qa" => Some(ProviderKind::Qa),
        _ => None,
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RunArgs {
    /// What to run in the background. Currently `search`.
    #[serde(default = "default_op")]
    op: String,
    /// Search kind: `web`, `code`, `docs`, or `qa`.
    kind: String,
    /// The query text.
    query: String,
    /// Max results (default 10, capped at 25).
    #[serde(default)]
    max_results: Option<usize>,
}

fn default_op() -> String {
    "search".to_string()
}

pub struct TaskRun;
impl Skill for TaskRun {
    fn name(&self) -> &'static str {
        "task_run"
    }
    fn description(&self) -> &'static str {
        "Start a long operation in the BACKGROUND and get a task id immediately; poll task_result \
        later (works on any client). Currently runs a search: op=\"search\", kind=web|code|docs|qa, \
        query=…. Lets the model fan out several searches at once instead of blocking on each."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RunArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<RunArgs>()?;
            if !args.op.trim().eq_ignore_ascii_case("search") {
                return Err(invalid("unsupported op (only \"search\" is supported)"));
            }
            let kind =
                parse_kind(&args.kind).ok_or_else(|| invalid("kind must be web/code/docs/qa"))?;
            let query = args.query.trim().to_string();
            if query.is_empty() {
                return Err(invalid("empty query"));
            }
            let limit = args.max_results.unwrap_or(10).clamp(1, 25);
            let label = format!(
                "{} search: {}",
                args.kind.trim().to_ascii_lowercase(),
                query
            );
            let id = server.tasks.create(label);

            let tasks = server.tasks.clone();
            let registry = server.registry.clone();
            let http = server.http.clone();
            let id_for_task = id.clone();
            let handle = tokio::spawn(async move {
                let q = SearchQuery {
                    text: query,
                    language: None,
                    site: None,
                    limit,
                    render: false,
                };
                let (hits, engine) = registry.search(kind, &http, &q).await;
                let out = if hits.is_empty() {
                    "No results.".to_string()
                } else {
                    format_hits(&hits, &engine)
                };
                tasks.finish(&id_for_task, Ok(out));
            });
            server.tasks.attach_abort(&id, handle.abort_handle());

            Ok(text_result(format!(
                "Started {id} ({}). Poll with task_result {{ id: \"{id}\" }}.",
                args.kind.trim().to_ascii_lowercase()
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IdArgs {
    /// The task id returned by task_run.
    id: String,
}

pub struct TaskList;
impl Skill for TaskList {
    fn name(&self) -> &'static str {
        "task_list"
    }
    fn description(&self) -> &'static str {
        "List background tasks (id, status, label, age), newest first."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(ctx.server.tasks.list())) })
    }
}

pub struct TaskStatus;
impl Skill for TaskStatus {
    fn name(&self) -> &'static str {
        "task_status"
    }
    fn description(&self) -> &'static str {
        "Report one background task's status (running/done/failed/cancelled) without its result."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<IdArgs>()?;
            match server.tasks.status(&args.id) {
                Some(s) => Ok(text_result(s)),
                None => Err(invalid(format!("no task '{}'", args.id))),
            }
        })
    }
}

pub struct TaskResult;
impl Skill for TaskResult {
    fn name(&self) -> &'static str {
        "task_result"
    }
    fn description(&self) -> &'static str {
        "Get a background task's result. If still running, says so (poll again); if done, returns \
        the output; if failed, returns the error."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<IdArgs>()?;
            let (status, label, result) = server
                .tasks
                .result(&args.id)
                .ok_or_else(|| invalid(format!("no task '{}'", args.id)))?;
            let body = match status {
                Status::Running => format!("{} is still running ({label}); poll again.", args.id),
                Status::Cancelled => format!("{} was cancelled ({label}).", args.id),
                Status::Failed => format!("{} failed ({label}):\n{result}", args.id),
                Status::Done => result,
            };
            Ok(text_result(body))
        })
    }
}

pub struct TaskCancel;
impl Skill for TaskCancel {
    fn name(&self) -> &'static str {
        "task_cancel"
    }
    fn description(&self) -> &'static str {
        "Cancel a running background task (no effect if it already finished)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<IdArgs>()?;
            match server.tasks.cancel(&args.id) {
                Some(true) => Ok(text_result(format!("Cancelled {}.", args.id))),
                Some(false) => Ok(text_result(format!("{} was not running.", args.id))),
                None => Err(invalid(format!("no task '{}'", args.id))),
            }
        })
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(TaskRun),
        Box::new(TaskList),
        Box::new(TaskStatus),
        Box::new(TaskResult),
        Box::new(TaskCancel),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_create_finish_result() {
        let t = Tasks::new();
        let id = t.create("web search: rust".into());
        assert!(t.status(&id).unwrap().contains("running"));
        t.finish(&id, Ok("3 results".into()));
        let (status, _, result) = t.result(&id).unwrap();
        assert_eq!(status.as_str(), "done");
        assert_eq!(result, "3 results");
    }

    #[test]
    fn cancel_marks_cancelled_then_no_op() {
        let t = Tasks::new();
        let id = t.create("docs search: x".into());
        assert_eq!(t.cancel(&id), Some(true));
        assert!(t.status(&id).unwrap().contains("cancelled"));
        // A finish after cancel is ignored.
        t.finish(&id, Ok("late".into()));
        let (status, _, _) = t.result(&id).unwrap();
        assert_eq!(status.as_str(), "cancelled");
        // Cancelling a non-running job reports false.
        assert_eq!(t.cancel(&id), Some(false));
        assert_eq!(t.cancel("task-999"), None);
    }

    #[test]
    fn parse_kind_variants() {
        assert!(matches!(parse_kind("web"), Some(ProviderKind::Web)));
        assert!(matches!(parse_kind("QA"), Some(ProviderKind::Qa)));
        assert!(parse_kind("nope").is_none());
    }

    #[test]
    fn evicts_oldest_finished_over_capacity() {
        let t = Tasks::new();
        let mut ids = Vec::new();
        for i in 0..(MAX_JOBS + 5) {
            let id = t.create(format!("job {i}"));
            t.finish(&id, Ok("ok".into()));
            ids.push(id);
        }
        let inner = t.0.lock().unwrap();
        assert!(inner.jobs.len() <= MAX_JOBS);
    }
}
