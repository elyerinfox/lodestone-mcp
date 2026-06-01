//! Async search launcher — `search_async`. **Off by default** (`[tasks].enabled`).
//!
//! Single tool: starts a background search (`web`/`code`/`docs`/`qa`) and
//! returns a `task_id` immediately. Management — listing, polling, fetching
//! results, cancelling — goes through the MCP-spec [`crate::skills::mcp_tasks`]
//! tools (`tasks_list` / `tasks_get` / `tasks_result` / `tasks_cancel`),
//! which read the shared [`crate::tasks::TaskRuntime`] registry. The same
//! runtime backs `mqtt_listen` / `meshtastic_listen`, so the model can
//! manage every backgrounded job (search, MQTT subscription, mesh listen)
//! through one inspection surface.
//!
//! Why a domain-specific launcher? The MCP Tasks spec (2025-11-25) envisions
//! launching via `tools/call` with `_meta.taskMode = "augment"` — the server
//! returns a `taskId` and runs the call in the background. rmcp 1.7 doesn't
//! handle that dispatch flag yet, so we expose explicit launcher tools
//! (`search_async`, `mqtt_listen`, `meshtastic_listen`) until it does.
//!
//! Module file historically named `tasks.rs` for the legacy `task_*` (singular)
//! family it housed; that family was collapsed into the runtime — only the
//! search launcher remains here.

use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::provider::{ProviderKind, SearchQuery, SearchResult};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

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
struct SearchAsyncArgs {
    /// Search kind: `web`, `code`, `docs`, or `qa`.
    kind: String,
    /// The query text.
    query: String,
    /// Max results (default 10, capped at 25).
    #[serde(default)]
    max_results: Option<usize>,
}

pub struct SearchAsync;
impl Skill for SearchAsync {
    fn name(&self) -> &'static str {
        "search_async"
    }
    fn description(&self) -> &'static str {
        "Start a search (`kind` = web/code/docs/qa) as a background task and get a \
        `task_id` immediately. Lets the model fan out several searches at once \
        instead of blocking on each. Poll via `tasks_result`, list via `tasks_list`, \
        cancel via `tasks_cancel`. Calls that include `_meta.progressToken` also \
        receive `notifications/progress` (engine start, partial counts) and \
        `notifications/tasks/status` on completion."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SearchAsyncArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let peer = ctx.peer.clone();
            let token = ctx.progress_token();
            let (server, args) = ctx.parse::<SearchAsyncArgs>()?;
            let kind =
                parse_kind(&args.kind).ok_or_else(|| invalid("kind must be web/code/docs/qa"))?;
            let query = args.query.trim().to_string();
            if query.is_empty() {
                return Err(invalid("empty query"));
            }
            let limit = args.max_results.unwrap_or(10).clamp(1, 25);
            let kind_lower = args.kind.trim().to_ascii_lowercase();
            let label = format!("{kind_lower} search: {query}");
            let registry = server.registry.clone();
            let http = server.http.clone();
            let runtime = server.task_runtime.clone();
            let runtime_for_observers = runtime.clone();
            let task_id = runtime
                .spawn("search_async", label, move |handle| async move {
                    // One progress tick at start so notification-capable
                    // clients see "search started" before the engines return.
                    handle
                        .progress(0.0, Some(1.0), Some("searching…".to_string()))
                        .await;
                    let q = SearchQuery {
                        text: query,
                        language: None,
                        site: None,
                        limit,
                        render: false,
                    };
                    // Race against cancellation; the in-flight search future
                    // won't be dropped early, but the body returns immediately
                    // and the runtime moves the task to Cancelled.
                    let cancel = handle.cancel_token();
                    let formatted = tokio::select! {
                        _ = cancel.cancelled() => {
                            return Err(anyhow!("cancelled"));
                        }
                        (hits, engine) = registry.search(kind, &http, &q) => {
                            let body = if hits.is_empty() {
                                "No results.".to_string()
                            } else {
                                format_hits(&hits, &engine)
                            };
                            handle
                                .progress(
                                    1.0,
                                    Some(1.0),
                                    Some(format!("{} hits via {engine}", hits.len())),
                                )
                                .await;
                            body
                        }
                    };
                    Ok(Value::String(formatted))
                })
                .await;
            // Wire the caller's progressToken (if any) and the peer so
            // every status change reaches them — same as the listen tools.
            if let (Some(p), Some(t)) = (peer.clone(), token) {
                runtime_for_observers.observe_progress(&task_id, p, t).await;
            }
            if let Some(p) = peer {
                runtime_for_observers.observe_status(&task_id, p).await;
            }
            Ok(text_result(format!(
                "Started {task_id} ({kind_lower}). Fetch with `tasks_result \
                 {{\"task_id\":\"{task_id}\"}}`; cancel with `tasks_cancel`."
            )))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(SearchAsync)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kind_variants() {
        assert!(matches!(parse_kind("web"), Some(ProviderKind::Web)));
        assert!(matches!(parse_kind("QA"), Some(ProviderKind::Qa)));
        assert!(parse_kind("nope").is_none());
    }

    #[test]
    fn format_hits_compact_layout() {
        let hits = vec![
            SearchResult {
                title: "First".into(),
                url: "https://a/".into(),
                snippet: "snip one".into(),
                ..Default::default()
            },
            SearchResult {
                title: "Second".into(),
                url: String::new(),
                snippet: String::new(),
                ..Default::default()
            },
        ];
        let body = format_hits(&hits, "engineX");
        assert!(body.starts_with("2 result(s) via engineX:"));
        assert!(body.contains("1. First"));
        assert!(body.contains("https://a/"));
        assert!(body.contains("snip one"));
        assert!(body.contains("2. Second"));
    }
}
