//! Global task runtime — the MCP **Tasks** primitive (2025-11-25 revision)
//! mirrored Lodestone-side so every skill can spawn long work that
//!   * returns a `taskId` immediately,
//!   * streams progress via `notifications/progress`,
//!   * pushes lifecycle changes via `notifications/tasks/status`
//!     (a CustomNotification — rmcp 1.7 doesn't ship the strongly-typed
//!     variant yet but `ServerNotification::CustomNotification`
//!     accepts arbitrary methods, so we use it without patching the
//!     crate; same wire bytes a typed variant would emit),
//!   * is fetchable via `tasks_get` / `tasks_result` / `tasks_list`,
//!   * is cancellable via `tasks_cancel` (also fires a status push).
//!
//! Held as `Arc<TaskRuntime>` on [`crate::Lodestone`] so every skill,
//! the WebSocket feed, and the dispatch wrapper see one shared
//! registry. Bounded (older finished tasks evict so a runaway
//! producer can't exhaust the host). Distinct from the legacy
//! [`crate::skills::tasks::Tasks`] (a polling-only search-result
//! buffer) — the legacy struct keeps its `task_*` tool surface for
//! back-compat; new work registers here.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rmcp::model::{
    CustomNotification, NumberOrString, ProgressNotificationParam, ProgressToken,
    ServerNotification,
};
use rmcp::service::Peer;
use rmcp::RoleServer;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// Bound on simultaneously-tracked tasks. Once exceeded, the oldest
/// finished tasks evict (running tasks are never evicted).
pub const MAX_TASKS: usize = 256;

/// Per-task cap on progress entries retained for `tasks_result` replay.
const PROGRESS_LOG_CAPACITY: usize = 128;

/// Lifecycle states mirroring the 2025-11-25 MCP Tasks spec. Serialized
/// to camelCase strings because that's what the spec wire bytes use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskStatus {
    Working,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Working => "working",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
    pub fn is_terminal(self) -> bool {
        !matches!(self, TaskStatus::Working)
    }
}

/// Snapshot of a task suitable for `tasks_get` / `tasks_list` responses.
#[derive(Debug, Clone, Serialize)]
pub struct TaskInfo {
    pub task_id: String,
    pub kind: String,
    pub label: String,
    pub status: TaskStatus,
    pub created_unix: u64,
    pub updated_unix: u64,
    /// Last reported progress (0..=total) — `None` until the task emits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    /// Total work units, if the task declared one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    /// Most recent progress message the task pushed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
}

/// Full result payload for `tasks_result`. Replays the progress log so a
/// late observer can reconstruct what happened.
#[derive(Debug, Clone, Serialize)]
pub struct TaskResultData {
    pub task_id: String,
    pub status: TaskStatus,
    /// Body the task returned on success. `None` for not-yet-finished or failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// One-line error message if the task failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Progress events emitted, oldest first.
    pub progress_log: Vec<ProgressEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEntry {
    pub at_unix_ms: i64,
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// A registered observer that wants `notifications/progress` for a task.
struct ProgressObserver {
    peer: Peer<RoleServer>,
    token: ProgressToken,
}

/// A registered observer that wants `notifications/tasks/status` for a task.
struct StatusObserver {
    peer: Peer<RoleServer>,
}

struct Task {
    info: TaskInfo,
    result: Option<Value>,
    error: Option<String>,
    progress_log: VecDeque<ProgressEntry>,
    cancel: CancellationToken,
    progress_observers: Vec<ProgressObserver>,
    status_observers: Vec<StatusObserver>,
}

struct Inner {
    tasks: HashMap<String, Task>,
    seq: u64,
}

/// Cloneable global runtime. Every clone shares the underlying state.
#[derive(Clone)]
pub struct TaskRuntime {
    inner: Arc<Mutex<Inner>>,
}

impl Default for TaskRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRuntime {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                tasks: HashMap::new(),
                seq: 1,
            })),
        }
    }

    /// Spawn a long-running task. The returned id is suitable for the
    /// `taskId` field in MCP `tools/call` responses + every `tasks_*`
    /// lookup. The body runs on the tokio runtime and receives a
    /// [`TaskHandle`] for emitting progress + checking for cancellation.
    ///
    /// On `Ok(value)` the task transitions to `Completed`; on `Err` to
    /// `Failed`. Either transition fires a `notifications/tasks/status`
    /// push to every registered status observer.
    pub async fn spawn<F, Fut, T>(
        &self,
        kind: impl Into<String>,
        label: impl Into<String>,
        body: F,
    ) -> String
    where
        F: FnOnce(TaskHandle) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T>> + Send + 'static,
        T: Serialize + Send + 'static,
    {
        let kind = kind.into();
        let label = label.into();
        let task_id = self.register(kind.clone(), label.clone()).await;
        let handle = TaskHandle {
            task_id: task_id.clone(),
            runtime: self.clone(),
            cancel: self.cancel_token(&task_id).await.unwrap_or_default(),
        };
        let rt = self.clone();
        let tid = task_id.clone();
        tokio::spawn(async move {
            let outcome = body(handle).await;
            match outcome {
                Ok(value) => {
                    let v = serde_json::to_value(value).unwrap_or(Value::Null);
                    rt.complete(&tid, v).await;
                }
                Err(e) => {
                    rt.fail(&tid, format!("{e:#}")).await;
                }
            }
        });
        task_id
    }

    async fn cancel_token(&self, task_id: &str) -> Option<CancellationToken> {
        let inner = self.inner.lock().await;
        inner.tasks.get(task_id).map(|t| t.cancel.clone())
    }

    /// Register a new task and return its id. Evicts the oldest finished
    /// tasks first when at capacity (never running ones).
    async fn register(&self, kind: String, label: String) -> String {
        let mut inner = self.inner.lock().await;
        if inner.tasks.len() >= MAX_TASKS {
            let mut finished: Vec<(String, u64)> = inner
                .tasks
                .values()
                .filter(|t| t.info.status.is_terminal())
                .map(|t| (t.info.task_id.clone(), t.info.updated_unix))
                .collect();
            finished.sort_by_key(|(_, t)| *t);
            let drop_n = (inner.tasks.len() + 1).saturating_sub(MAX_TASKS);
            for (tid, _) in finished.into_iter().take(drop_n) {
                inner.tasks.remove(&tid);
            }
        }
        let n = inner.seq;
        inner.seq += 1;
        let task_id = format!("task-{n}");
        let now = now_secs();
        inner.tasks.insert(
            task_id.clone(),
            Task {
                info: TaskInfo {
                    task_id: task_id.clone(),
                    kind,
                    label,
                    status: TaskStatus::Working,
                    created_unix: now,
                    updated_unix: now,
                    progress: None,
                    total: None,
                    last_message: None,
                },
                result: None,
                error: None,
                progress_log: VecDeque::with_capacity(PROGRESS_LOG_CAPACITY),
                cancel: CancellationToken::new(),
                progress_observers: Vec::new(),
                status_observers: Vec::new(),
            },
        );
        task_id
    }

    pub async fn get(&self, task_id: &str) -> Option<TaskInfo> {
        let inner = self.inner.lock().await;
        inner.tasks.get(task_id).map(|t| t.info.clone())
    }

    pub async fn list(&self) -> Vec<TaskInfo> {
        let inner = self.inner.lock().await;
        let mut out: Vec<TaskInfo> = inner.tasks.values().map(|t| t.info.clone()).collect();
        out.sort_by_key(|t| std::cmp::Reverse(t.updated_unix));
        out
    }

    pub async fn result(&self, task_id: &str) -> Option<TaskResultData> {
        let inner = self.inner.lock().await;
        let t = inner.tasks.get(task_id)?;
        Some(TaskResultData {
            task_id: t.info.task_id.clone(),
            status: t.info.status,
            result: t.result.clone(),
            error: t.error.clone(),
            progress_log: t.progress_log.iter().cloned().collect(),
        })
    }

    /// Mark a task cancelled. Returns true iff it was running.
    pub async fn cancel(&self, task_id: &str) -> bool {
        let (cancelled, observers, snapshot) = {
            let mut inner = self.inner.lock().await;
            let Some(t) = inner.tasks.get_mut(task_id) else {
                return false;
            };
            if t.info.status.is_terminal() {
                return false;
            }
            t.cancel.cancel();
            t.info.status = TaskStatus::Cancelled;
            t.info.updated_unix = now_secs();
            let snapshot = t.info.clone();
            let observers: Vec<Peer<RoleServer>> =
                t.status_observers.iter().map(|s| s.peer.clone()).collect();
            (true, observers, snapshot)
        };
        for peer in observers {
            push_task_status(&peer, &snapshot, None, None).await;
        }
        cancelled
    }

    /// Register an MCP client peer to receive `notifications/progress`
    /// for this task, identified by the progressToken the client put
    /// in the originating request's `_meta.progressToken`.
    pub async fn observe_progress(
        &self,
        task_id: &str,
        peer: Peer<RoleServer>,
        token: ProgressToken,
    ) -> bool {
        let mut inner = self.inner.lock().await;
        let Some(t) = inner.tasks.get_mut(task_id) else {
            return false;
        };
        t.progress_observers.push(ProgressObserver { peer, token });
        true
    }

    /// Register a peer to receive `notifications/tasks/status` pushes for
    /// this task on lifecycle transitions (completed / failed / cancelled).
    pub async fn observe_status(&self, task_id: &str, peer: Peer<RoleServer>) -> bool {
        let mut inner = self.inner.lock().await;
        let Some(t) = inner.tasks.get_mut(task_id) else {
            return false;
        };
        t.status_observers.push(StatusObserver { peer });
        true
    }

    /// Internal: push a progress event from inside a running task.
    /// Updates the task record, appends to the progress log (evicting
    /// the oldest if at cap), and fans out to every progress observer.
    async fn emit_progress(
        &self,
        task_id: &str,
        progress: f64,
        total: Option<f64>,
        message: Option<String>,
    ) {
        let (observers, snapshot_total) = {
            let mut inner = self.inner.lock().await;
            let Some(t) = inner.tasks.get_mut(task_id) else {
                return;
            };
            if t.info.status.is_terminal() {
                return;
            }
            t.info.progress = Some(progress);
            if total.is_some() {
                t.info.total = total;
            }
            if message.is_some() {
                t.info.last_message = message.clone();
            }
            t.info.updated_unix = now_secs();
            if t.progress_log.len() >= PROGRESS_LOG_CAPACITY {
                t.progress_log.pop_front();
            }
            t.progress_log.push_back(ProgressEntry {
                at_unix_ms: now_unix_ms(),
                progress,
                total: t.info.total,
                message: message.clone(),
            });
            let observers: Vec<(Peer<RoleServer>, ProgressToken)> = t
                .progress_observers
                .iter()
                .map(|o| (o.peer.clone(), o.token.clone()))
                .collect();
            (observers, t.info.total)
        };
        for (peer, token) in observers {
            let params = ProgressNotificationParam {
                progress_token: token,
                progress,
                total: snapshot_total,
                message: message.clone(),
            };
            if let Err(e) = peer.notify_progress(params).await {
                warn!(target: "tasks", task_id, error = %e, "progress notify failed");
            }
        }
    }

    async fn complete(&self, task_id: &str, value: Value) {
        let (status_obs, prog_obs, snapshot, final_progress, final_total) = {
            let mut inner = self.inner.lock().await;
            let Some(t) = inner.tasks.get_mut(task_id) else {
                return;
            };
            // Cancelled wins — the body may have finished racing the cancel signal.
            if t.info.status == TaskStatus::Cancelled {
                return;
            }
            t.info.status = TaskStatus::Completed;
            t.info.updated_unix = now_secs();
            t.result = Some(value.clone());
            (
                t.status_observers
                    .iter()
                    .map(|s| s.peer.clone())
                    .collect::<Vec<_>>(),
                t.progress_observers
                    .iter()
                    .map(|o| (o.peer.clone(), o.token.clone()))
                    .collect::<Vec<_>>(),
                t.info.clone(),
                t.info.progress.unwrap_or(1.0),
                t.info.total,
            )
        };
        // Final progress tick — surface "done" on clients that only watch progress.
        for (peer, token) in prog_obs {
            let params = ProgressNotificationParam {
                progress_token: token,
                progress: final_progress,
                total: final_total,
                message: Some("completed".into()),
            };
            let _ = peer.notify_progress(params).await;
        }
        // The lifecycle notification — task-aware clients react to this.
        for peer in status_obs {
            push_task_status(&peer, &snapshot, Some(value.clone()), None).await;
        }
    }

    async fn fail(&self, task_id: &str, error: String) {
        let (status_obs, snapshot) = {
            let mut inner = self.inner.lock().await;
            let Some(t) = inner.tasks.get_mut(task_id) else {
                return;
            };
            if t.info.status == TaskStatus::Cancelled {
                return;
            }
            t.info.status = TaskStatus::Failed;
            t.info.updated_unix = now_secs();
            t.error = Some(error.clone());
            (
                t.status_observers
                    .iter()
                    .map(|s| s.peer.clone())
                    .collect::<Vec<_>>(),
                t.info.clone(),
            )
        };
        for peer in status_obs {
            push_task_status(&peer, &snapshot, None, Some(error.clone())).await;
        }
    }
}

/// Skill-side handle: lets a running task body emit progress + check
/// for cancellation. Created by [`TaskRuntime::spawn`] and passed to
/// the body. Cheap to clone — internally holds an `Arc`-backed runtime
/// reference.
#[derive(Clone)]
pub struct TaskHandle {
    task_id: String,
    runtime: TaskRuntime,
    cancel: CancellationToken,
}

impl TaskHandle {
    #[allow(dead_code)] // public skill-side API
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Cancellation token. The runtime cancels it when `tasks_cancel`
    /// fires; the task body should respect it.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    #[allow(dead_code)] // public skill-side API
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Emit a progress update. `progress` is the current count; `total`
    /// is optional (omit for unbounded streams like an MQTT subscription).
    /// `message` is a one-line human-readable summary (e.g. the topic
    /// + a snippet of the payload).
    pub async fn progress(&self, progress: f64, total: Option<f64>, message: Option<String>) {
        self.runtime
            .emit_progress(&self.task_id, progress, total, message)
            .await;
    }
}

/// Build + send a `notifications/tasks/status` (the 2025-11-25 Tasks
/// completion-push notification). rmcp 1.7 doesn't ship a typed variant
/// so we use [`CustomNotification`] — exactly the same wire bytes a
/// typed variant would emit.
async fn push_task_status(
    peer: &Peer<RoleServer>,
    info: &TaskInfo,
    result: Option<Value>,
    error: Option<String>,
) {
    let mut task = json!({
        "taskId": info.task_id,
        "kind": info.kind,
        "label": info.label,
        "status": info.status,
        "createdAt": info.created_unix,
        "updatedAt": info.updated_unix,
    });
    if let Some(p) = info.progress {
        task["progress"] = json!(p);
    }
    if let Some(t) = info.total {
        task["total"] = json!(t);
    }
    if let Some(m) = &info.last_message {
        task["lastMessage"] = json!(m);
    }
    if let Some(r) = result {
        task["result"] = r;
    }
    if let Some(e) = error {
        task["error"] = json!(e);
    }
    let params = json!({ "task": task });
    let note = CustomNotification::new("notifications/tasks/status", Some(params));
    if let Err(e) = peer
        .send_notification(ServerNotification::CustomNotification(note))
        .await
    {
        warn!(
            target: "tasks",
            task_id = %info.task_id,
            error = %e,
            "notifications/tasks/status delivery failed"
        );
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Parse an MCP `progressToken` out of an arbitrary JSON value (`_meta`
/// from a tool call). Handles both string and number variants per spec.
#[allow(dead_code)] // utility for skills that build their own non-MCP request shape
pub fn parse_progress_token(meta: &Value) -> Option<ProgressToken> {
    let raw = meta.get("progressToken")?;
    if let Some(s) = raw.as_str() {
        return Some(ProgressToken(NumberOrString::String(s.to_string().into())));
    }
    if let Some(i) = raw.as_i64() {
        return Some(ProgressToken(NumberOrString::Number(i)));
    }
    if let Some(u) = raw.as_u64() {
        if u <= i64::MAX as u64 {
            return Some(ProgressToken(NumberOrString::Number(u as i64)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn spawn_completes_with_result() {
        let rt = TaskRuntime::new();
        let id = rt
            .spawn("test", "echo", |_h| async move {
                Ok::<_, anyhow::Error>(json!({"x": 1}))
            })
            .await;
        // Yield until the body completes.
        for _ in 0..20 {
            if let Some(info) = rt.get(&id).await {
                if info.status == TaskStatus::Completed {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let r = rt.result(&id).await.unwrap();
        assert_eq!(r.status, TaskStatus::Completed);
        assert_eq!(r.result, Some(json!({"x": 1})));
    }

    #[tokio::test]
    async fn cancellation_terminates_running_task() {
        let rt = TaskRuntime::new();
        let id = rt
            .spawn("test", "wait", |h| async move {
                h.cancel_token().cancelled().await;
                Ok::<_, anyhow::Error>(json!("never"))
            })
            .await;
        // Let the spawn body schedule.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(rt.cancel(&id).await);
        assert_eq!(rt.get(&id).await.unwrap().status, TaskStatus::Cancelled);
        // Second cancel is a no-op.
        assert!(!rt.cancel(&id).await);
    }

    #[tokio::test]
    async fn progress_log_records_messages() {
        let rt = TaskRuntime::new();
        let id = rt
            .spawn("test", "ticker", |h| async move {
                for i in 0..3 {
                    h.progress(i as f64, Some(3.0), Some(format!("tick {i}")))
                        .await;
                }
                Ok::<_, anyhow::Error>(json!("done"))
            })
            .await;
        for _ in 0..50 {
            if let Some(info) = rt.get(&id).await {
                if info.status == TaskStatus::Completed {
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let r = rt.result(&id).await.unwrap();
        assert_eq!(r.progress_log.len(), 3);
        assert_eq!(r.progress_log[0].message.as_deref(), Some("tick 0"));
        assert_eq!(r.progress_log[2].message.as_deref(), Some("tick 2"));
    }

    #[test]
    fn parse_token_number_or_string() {
        let s = parse_progress_token(&json!({"progressToken": "abc"})).unwrap();
        assert!(matches!(s.0, NumberOrString::String(_)));
        let n = parse_progress_token(&json!({"progressToken": 42})).unwrap();
        assert!(matches!(n.0, NumberOrString::Number(42)));
        assert!(parse_progress_token(&json!({})).is_none());
    }
}
