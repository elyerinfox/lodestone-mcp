//! Long-lived browser sessions — one Chromium process shared across the
//! server, but each session gets its own isolated BrowserContext (separate
//! cookies / local storage / cache) so multiple concurrent flows don't
//! step on each other.
//!
//! The session manager keeps a `Page` (one tab) alive across tool calls,
//! addressed by an opaque `session_id`. Tools (`browser_open`,
//! `browser_navigate`, etc.) pass the id back and the manager looks up
//! the right page. An idle reaper closes sessions inactive for more
//! than `idle_timeout_secs` so a forgotten session doesn't pin a
//! BrowserContext forever.
//!
//! v1 scope (this module): foundation only — `browser_open`,
//! `browser_navigate`, `browser_close`, `browser_list`. No observation
//! engine yet; navigate / open just return the current URL + title.
//! Interaction (click / type / wait / extract) and the observation
//! engine (compact DOM tree + screenshot) ship in follow-up commits.
//!
//! Everything here is **local-only**. Sessions don't migrate over the
//! constellation, browser bytes don't enter the gossip cache, and
//! [network] auth doesn't gate these tools (they're MCP tools, gated
//! by the MCP `auth_token` like every other tool).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chromiumoxide::cdp::browser_protocol::target::CreateTargetParams;
use chromiumoxide::Page;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use tokio::sync::{Mutex, OnceCell, RwLock};

use crate::browser::shared_global;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid};

pub const TOOL_NAMES: &[&str] = &[
    "browser_open",
    "browser_navigate",
    "browser_close",
    "browser_list",
];

/// Runtime-tunable knobs for the session manager. Exposed read/write
/// via `/api/settings/browser` in a later commit; here they only have
/// the in-memory representation + a default.
#[derive(Debug, Clone)]
pub struct BrowserSessionConfig {
    /// Close a session that hasn't been touched for this long. Default
    /// 1800 (30 min) — long enough to survive a model think-loop,
    /// short enough that an abandoned flow doesn't pin a context for
    /// hours.
    pub idle_timeout_secs: u64,
    /// Cap on simultaneously-open sessions. `browser_open` returns an
    /// error past this point so we don't OOM the host with a
    /// runaway loop. Default 8.
    pub max_concurrent: usize,
}

impl Default for BrowserSessionConfig {
    fn default() -> Self {
        Self { idle_timeout_secs: 1800, max_concurrent: 8 }
    }
}

/// One open tab. `page` is the chromiumoxide handle; `context_id` lets
/// us dispose the context on close (which frees Chrome's per-context
/// allocations — closing only the page leaks the context).
///
/// `serial` is the per-session lock every tool grabs. Chromium itself
/// is fine with parallel CDP calls on one page, but for the model's
/// mental model "two tools running on the same session interleave their
/// effects" is worse than "the second tool waits for the first." The
/// lock is awaited, not contended, so the cost is near-zero in the
/// common (serial) case.
struct Session {
    id: String,
    context_id: String,
    page: Page,
    created_at_ms: i64,
    last_used_ms: AtomicI64,
    serial: Mutex<()>,
}

impl Session {
    fn touch(&self) {
        self.last_used_ms.store(now_ms(), Ordering::Relaxed);
    }
}

pub struct BrowserSessionManager {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    cfg: RwLock<BrowserSessionConfig>,
}

impl BrowserSessionManager {
    pub fn new(cfg: BrowserSessionConfig) -> Arc<Self> {
        let m = Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            cfg: RwLock::new(cfg),
        });
        spawn_reaper(m.clone());
        m
    }

    pub async fn config(&self) -> BrowserSessionConfig {
        self.cfg.read().await.clone()
    }

    pub async fn open(&self) -> Result<(String, String, String), McpError> {
        let cfg = self.config().await;
        {
            let table = self.sessions.read().await;
            if table.len() >= cfg.max_concurrent {
                return Err(invalid(format!(
                    "max_concurrent sessions reached ({}); close one with browser_close",
                    cfg.max_concurrent
                )));
            }
        }
        let renderer = shared_global();
        let handle = renderer
            .handle_for_session()
            .await
            .map_err(|e| internal(anyhow::anyhow!("browser unavailable: {e}")))?;
        let browser = handle.as_chromiumoxide();
        let ctx_id = browser
            .create_browser_context(
                chromiumoxide::cdp::browser_protocol::target::CreateBrowserContextParams::default(),
            )
            .await
            .map_err(|e| internal(anyhow::anyhow!("create_browser_context: {e}")))?;
        let mut params: CreateTargetParams = "about:blank".into();
        params.browser_context_id = Some(ctx_id.clone());
        let page = browser
            .new_page(params)
            .await
            .map_err(|e| internal(anyhow::anyhow!("new_page: {e}")))?;
        let id = new_session_id();
        let now = now_ms();
        let url = page.url().await.unwrap_or_default().unwrap_or_default();
        let session = Arc::new(Session {
            id: id.clone(),
            context_id: ctx_id.inner().clone(),
            page,
            created_at_ms: now,
            last_used_ms: AtomicI64::new(now),
            serial: Mutex::new(()),
        });
        self.sessions.write().await.insert(id.clone(), session);
        Ok((id, url, String::new()))
    }

    pub async fn navigate(
        &self,
        session_id: &str,
        url: &str,
    ) -> Result<(String, String), McpError> {
        let session = self.lookup(session_id).await?;
        let _g = session.serial.lock().await;
        session.touch();
        session
            .page
            .goto(url)
            .await
            .map_err(|e| invalid(format!("goto {url}: {e}")))?;
        // Best-effort: wait_for_navigation can stall on long-tail trackers.
        // We bound the wait at 15s and return whatever state we ended in.
        let _ = tokio::time::timeout(
            Duration::from_secs(15),
            session.page.wait_for_navigation(),
        )
        .await;
        let final_url = session
            .page
            .url()
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        let title = session
            .page
            .get_title()
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        session.touch();
        Ok((final_url, title))
    }

    pub async fn close(&self, session_id: &str) -> Result<(), McpError> {
        let session = {
            let mut table = self.sessions.write().await;
            table.remove(session_id).ok_or_else(|| {
                invalid(format!("unknown session_id: {session_id}"))
            })?
        };
        // Dispose the context — that closes every page belonging to it
        // (per chromium's `Target.disposeBrowserContext` docs) without
        // firing beforeunload hooks, and frees the per-context
        // allocations. Errors here are logged but not bubbled up —
        // once we removed the entry the caller's session is gone.
        if let Ok(handle) = shared_global().handle_for_session().await {
            if let Err(e) = handle
                .as_chromiumoxide()
                .dispose_browser_context(session.context_id.clone())
                .await
            {
                tracing::warn!(session_id = %session.id, error = %e, "dispose_browser_context");
            }
        }
        Ok(())
    }

    pub async fn list(&self) -> Vec<SessionSummary> {
        let table = self.sessions.read().await;
        let now = now_ms();
        let mut rows: Vec<SessionSummary> = table
            .values()
            .map(|s| {
                let last = s.last_used_ms.load(Ordering::Relaxed);
                SessionSummary {
                    session_id: s.id.clone(),
                    created_secs_ago: ((now - s.created_at_ms) / 1000).max(0) as u64,
                    idle_secs: ((now - last) / 1000).max(0) as u64,
                }
            })
            .collect();
        rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        rows
    }

    async fn lookup(&self, session_id: &str) -> Result<Arc<Session>, McpError> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown session_id: {session_id}")))
    }
}

/// Listing row for `browser_list` and the dashboard. URL + title are
/// added in the interaction commit so the listing reflects the live
/// page state; for v1 we only carry session bookkeeping.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub created_secs_ago: u64,
    pub idle_secs: u64,
}

// ---------------------------------------------------------------------------
// Reaper
// ---------------------------------------------------------------------------

fn spawn_reaper(manager: Arc<BrowserSessionManager>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let timeout = manager.config().await.idle_timeout_secs as i64 * 1000;
            let now = now_ms();
            let expired: Vec<String> = {
                let table = manager.sessions.read().await;
                table
                    .values()
                    .filter(|s| now - s.last_used_ms.load(Ordering::Relaxed) >= timeout)
                    .map(|s| s.id.clone())
                    .collect()
            };
            for id in expired {
                tracing::info!(session_id = %id, "browser session idle-expired");
                let _ = manager.close(&id).await;
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Global accessor
// ---------------------------------------------------------------------------

static MANAGER: OnceCell<Arc<BrowserSessionManager>> = OnceCell::const_new();

/// Lazily-initialized process-wide session manager. The first caller
/// wins; later configure() calls (from dashboard settings) mutate the
/// existing manager's config in place rather than re-creating it.
pub async fn manager() -> Arc<BrowserSessionManager> {
    MANAGER
        .get_or_init(|| async { BrowserSessionManager::new(BrowserSessionConfig::default()) })
        .await
        .clone()
}

// ---------------------------------------------------------------------------
// id + clock helpers
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn new_session_id() -> String {
    // Hash (nanos, pid, atomic counter) to avoid pulling in `rand`/`hex`
    // crates the project doesn't already use. Same approach as
    // constellation's `random_id`; collisions vs. eight open sessions
    // are not a concern since the counter monotonically increases.
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("{nanos}-{}-{n}", std::process::id());
    format!("br_{}", &crate::constellation::hash_key(&seed)[..12])
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

fn text(s: impl Into<String>) -> CallToolResult {
    crate::text_result(s.into())
}

fn json(v: serde_json::Value) -> CallToolResult {
    crate::text_result(v.to_string())
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NoArgs {}

pub struct BrowserOpen;
impl Skill for BrowserOpen {
    fn name(&self) -> &'static str {
        "browser_open"
    }
    fn description(&self) -> &'static str {
        "Open a new persistent browser session (isolated cookies / local storage / cache). Returns \
         a `session_id` to pass to subsequent `browser_*` tools. The tab starts at `about:blank` — \
         use `browser_navigate` to go somewhere. Sessions auto-close after the idle timeout \
         (default 30 min) or when `browser_close` is called. The server caps concurrent sessions \
         (default 8); past the cap, `browser_open` returns an error and you must close one first."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let _ = ctx.parse::<NoArgs>()?;
            let mgr = manager().await;
            let (id, url, _) = mgr.open().await?;
            Ok(json(serde_json::json!({ "session_id": id, "url": url })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NavigateArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
    /// URL to navigate to. `about:` and `chrome://` URLs are allowed but
    /// most third-party trackers / extensions are not loaded (we run
    /// headless without persistent profile).
    url: String,
}

pub struct BrowserNavigate;
impl Skill for BrowserNavigate {
    fn name(&self) -> &'static str {
        "browser_navigate"
    }
    fn description(&self) -> &'static str {
        "Navigate an existing browser session to a URL. Waits up to 15s for the navigation to \
         settle, then returns the final URL after redirects + the page title. State (cookies, \
         local storage, scroll position) is preserved across the navigation just like a real tab."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NavigateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<NavigateArgs>()?;
            let mgr = manager().await;
            let (url, title) = mgr.navigate(&args.session_id, &args.url).await?;
            Ok(json(serde_json::json!({ "url": url, "title": title })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CloseArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
}

pub struct BrowserClose;
impl Skill for BrowserClose {
    fn name(&self) -> &'static str {
        "browser_close"
    }
    fn description(&self) -> &'static str {
        "Close a browser session, disposing its tab and isolated context (cookies/localStorage \
         are freed). Idempotent in spirit but errors on an unknown session_id — confirm via \
         `browser_list` if uncertain."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CloseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<CloseArgs>()?;
            let mgr = manager().await;
            mgr.close(&args.session_id).await?;
            Ok(text(format!("closed session {}", args.session_id)))
        })
    }
}

pub struct BrowserList;
impl Skill for BrowserList {
    fn name(&self) -> &'static str {
        "browser_list"
    }
    fn description(&self) -> &'static str {
        "List every open browser session: session_id, how long ago it was opened, and how long \
         it's been idle. Use this to find a lingering session before opening a new one (the \
         concurrent cap is shared)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let _ = ctx.parse::<NoArgs>()?;
            let mgr = manager().await;
            let rows = mgr.list().await;
            Ok(json(serde_json::json!({ "sessions": rows })))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(BrowserOpen),
        Box::new(BrowserNavigate),
        Box::new(BrowserClose),
        Box::new(BrowserList),
    ]
}
