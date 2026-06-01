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

#[allow(dead_code)]
pub const TOOL_NAMES: &[&str] = &[
    "browser_open",
    "browser_navigate",
    "browser_close",
    "browser_list",
    "browser_click",
    "browser_type",
    "browser_wait",
    "browser_extract",
    "browser_eval",
    "browser_screenshot",
    "browser_persona_get",
    "browser_persona_list",
    "browser_persona_reset",
    "browser_persona_delegate",
];

// ---------------------------------------------------------------------------
// Named-session personas (#127)
// ---------------------------------------------------------------------------
//
// A persona is a long-lived, NAMED browser session that providers
// (and the model) route through to ACCUMULATE warm state — cookies,
// solved-CAPTCHA tokens, fingerprint — for one specific site or
// vendor. Hitting `google.com` through the same persona 50 times in a
// row looks like one persistent user; spinning up 50 fresh contexts
// looks like a bot and gets rate-limited.
//
// Personas have a small state machine the operator can observe and act
// on:
//
//   Healthy   normal use — every action goes through.
//   Suspect   the detector flagged something (CAPTCHA selector,
//             429/403 page, "challenge" in the URL). Outbound use
//             still works but the dashboard shows the warning.
//   Blocked   second strike. Calls return an error until the
//             operator confirms a reset from the dashboard.
//
// Reset = dispose the persona's session + create a fresh one (fresh
// context, fresh cookies). State returns to Healthy. The auto-flip
// is conservative; the human-in-the-loop reset is what `BrowserPersona`
// is built around.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PersonaState {
    Healthy,
    Suspect,
    Blocked,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PersonaSummary {
    pub name: String,
    pub state: PersonaState,
    pub session_id: Option<String>,
    pub url: Option<String>,
    pub last_warning: Option<String>,
    pub age_secs: u64,
}

struct Persona {
    name: String,
    session_id: tokio::sync::RwLock<Option<String>>,
    state: tokio::sync::RwLock<PersonaState>,
    strikes: AtomicI64,
    last_warning: tokio::sync::RwLock<Option<String>>,
    created_at_ms: i64,
    /// Wall-clock ms of the most recent touch (any persona op). Used by
    /// the persona reaper to drop orphaned per-peer personas whose owner has
    /// gone away without explicitly closing their delegated session.
    last_touched_ms: AtomicI64,
}

impl Persona {
    fn new(name: String) -> Self {
        let now = now_ms();
        Self {
            name,
            session_id: tokio::sync::RwLock::new(None),
            state: tokio::sync::RwLock::new(PersonaState::Healthy),
            strikes: AtomicI64::new(0),
            last_warning: tokio::sync::RwLock::new(None),
            created_at_ms: now,
            last_touched_ms: AtomicI64::new(now),
        }
    }
    fn touch(&self) {
        self.last_touched_ms.store(now_ms(), Ordering::Relaxed);
    }
}

/// Per-peer key for a guest session — peer A's `google` and peer B's
/// `google` are isolated browser contexts (separate cookies, separate
/// fingerprint). Used as the map key in `guest_sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GuestKey {
    peer_id: String,
    persona_name: String,
}

/// One tab we're hosting on behalf of a constellation peer. The peer
/// asked us to drive their `google` (or whatever) persona, so the
/// cookies + warm state accumulate UNDER THEIR OWNERSHIP — when they
/// leave the constellation we tear it down. Distinct concept from a
/// LOCAL persona which the model owns and which we never auto-reap.
///
/// Implementation-wise this is just a `Persona` plus the owner key;
/// keeping it a separate type makes the dashboard split (your
/// personas vs hosted for peers) self-documenting at the type level
/// instead of relying on a naming convention.
struct GuestSession {
    key: GuestKey,
    persona: Arc<Persona>,
}

/// Dashboard row for a guest session. Distinct shape from
/// `PersonaSummary` so the frontend can render two tables without
/// having to branch on a kind discriminator.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GuestSessionSummary {
    /// node_id of the peer that owns this session's state.
    pub peer_id: String,
    /// Bare persona name the peer asked for (`"google"`, etc.) — same
    /// label the peer would call it on its own end.
    pub persona_name: String,
    pub state: PersonaState,
    pub session_id: Option<String>,
    pub url: Option<String>,
    pub last_warning: Option<String>,
    pub age_secs: u64,
}

/// What the model wants back after an action.
///
/// - `None`: just the action's direct result (url / matched / values).
/// - `Tree`: a compact list of interactive elements with stable
///   selectors — cheap, text-only, the default reactive surface.
/// - `Screenshot`: a base64-encoded PNG of the viewport.
/// - `Both`: tree + screenshot.
///
/// Tools that take this default to `None` so a multi-step flow doesn't
/// pay the observation cost every step. Pass `tree` after the action
/// where the model needs to decide what to do next.
#[derive(Debug, Clone, Copy, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum ObserveMode {
    #[default]
    None,
    Tree,
    Screenshot,
    Both,
}

/// What the manager hands back to a tool when the caller asks to
/// observe. `tree` is a `Vec` of interactive-element rows; only the
/// fields actually populated are serialized so the wire shape stays
/// compact. `screenshot_b64` is a base64 PNG of the viewport.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Observation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<Vec<TreeNode>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot_b64: Option<String>,
}

/// One row in the compact accessibility-style tree. `selector` is what
/// the model passes to `browser_click` / `browser_type` to act on this
/// element. `role` is the element's effective ARIA role (or tag-name
/// for non-ARIA elements like `a`/`button`/`input`). `name` is the
/// element's accessible name — aria-label if set, otherwise trimmed
/// inner text. `value` is populated for inputs.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TreeNode {
    pub role: String,
    pub name: String,
    pub selector: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

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
        Self {
            idle_timeout_secs: 1800,
            max_concurrent: 8,
        }
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
    /// SSRF guard switch. `false` for the model's own
    /// `browser_open` (the operator opted in to running tools locally
    /// so we don't restrict them). `true` for sessions created on
    /// behalf of a constellation peer via `/constellation/browser_persona`
    /// (#128) — every navigation goes through
    /// [`crate::skills::ssrf::assert_public`] to refuse local-network
    /// hosts.
    restrict_to_public: bool,
}

impl Session {
    fn touch(&self) {
        self.last_used_ms.store(now_ms(), Ordering::Relaxed);
    }
}

pub struct BrowserSessionManager {
    sessions: RwLock<HashMap<String, Arc<Session>>>,
    cfg: RwLock<BrowserSessionConfig>,
    /// LOCAL personas — warm browser identities the model owns. Bare
    /// names (`"google"`, `"github"`). Never auto-reaped; operator
    /// owns the lifecycle via the dashboard or
    /// `browser_persona_reset`.
    personas: RwLock<HashMap<String, Arc<Persona>>>,
    /// GUEST sessions — tabs we host for constellation peers. Keyed
    /// by `(peer_id, persona_name)` so peer A's `google` and peer B's
    /// `google` are different browser contexts. SSRF-restricted at
    /// the session level. Reaped when the peer leaves the
    /// constellation, or when idle past `idle_timeout_secs * 2`.
    /// Never visible to the local model's `browser_persona_*` tools.
    guest_sessions: RwLock<HashMap<GuestKey, Arc<GuestSession>>>,
    /// Reverse index session_id → owner. The detector consults this
    /// to know which kind of warm slot to report a CAPTCHA / block
    /// warning against.
    session_owner: RwLock<HashMap<String, SessionOwner>>,
}

#[derive(Debug, Clone)]
enum SessionOwner {
    Local(String),
    Guest(GuestKey),
}

impl BrowserSessionManager {
    pub fn new(cfg: BrowserSessionConfig) -> Arc<Self> {
        let m = Arc::new(Self {
            sessions: RwLock::new(HashMap::new()),
            cfg: RwLock::new(cfg),
            personas: RwLock::new(HashMap::new()),
            guest_sessions: RwLock::new(HashMap::new()),
            session_owner: RwLock::new(HashMap::new()),
        });
        spawn_reaper(m.clone());
        m
    }

    pub async fn config(&self) -> BrowserSessionConfig {
        self.cfg.read().await.clone()
    }

    pub async fn open(&self) -> Result<(String, String, String), McpError> {
        self.open_internal(false).await
    }

    /// Open a session that runs through the SSRF guard on every
    /// navigation. Used by the constellation-delegation path (#128)
    /// so a peer can drive our browser without being able to
    /// enumerate our LAN or hit cloud-metadata endpoints.
    pub async fn open_restricted(&self) -> Result<(String, String, String), McpError> {
        self.open_internal(true).await
    }

    async fn open_internal(
        &self,
        restrict_to_public: bool,
    ) -> Result<(String, String, String), McpError> {
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
            restrict_to_public,
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
        if session.restrict_to_public {
            crate::skills::ssrf::assert_public(url).await?;
        }
        let _g = session.serial.lock().await;
        session.touch();
        session
            .page
            .goto(url)
            .await
            .map_err(|e| invalid(format!("goto {url}: {e}")))?;
        // Best-effort: wait_for_navigation can stall on long-tail trackers.
        // We bound the wait at 15s and return whatever state we ended in.
        let _ =
            tokio::time::timeout(Duration::from_secs(15), session.page.wait_for_navigation()).await;
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
        // If this session is the live session for a persona, run the
        // heuristic detector on the post-navigation page. The
        // detector is cheap (URL + title pattern match) and reports
        // are advisory — the state machine handles the throttling.
        self.maybe_detect_poisoning(session_id, &final_url, &title)
            .await;
        Ok((final_url, title))
    }

    /// If `session_id` belongs to a persona, scan the URL + title for
    /// well-known CAPTCHA / block patterns and report a warning on the
    /// persona when one matches. False positives are tolerable — the
    /// operator just clicks "reset" in the dashboard.
    async fn maybe_detect_poisoning(&self, session_id: &str, url: &str, title: &str) {
        let owner = match self.session_owner.read().await.get(session_id).cloned() {
            Some(o) => o,
            None => return,
        };
        let lower_url = url.to_ascii_lowercase();
        let lower_title = title.to_ascii_lowercase();
        let url_signals = [
            "captcha",
            "challenge",
            "checkpoint",
            "blocked",
            "ratelimit",
            "rate-limit",
            "/verify",
        ];
        let title_signals = [
            "captcha",
            "are you a robot",
            "are you human",
            "just a moment",
            "checking your browser",
            "access denied",
            "403 forbidden",
            "429",
            "too many requests",
            "verify you are human",
            "attention required",
        ];
        let hit = url_signals
            .iter()
            .find(|s| lower_url.contains(*s))
            .or_else(|| title_signals.iter().find(|s| lower_title.contains(*s)));
        if let Some(hit) = hit {
            self.report_warning(&owner, &format!("matched signature {hit:?} in url/title"))
                .await;
        }
    }

    /// Mark a CAPTCHA / 429 / 403 hit against whichever warm-state
    /// slot owns this session. First strike → suspect, second →
    /// blocked. Routes to the local-persona registry or the guest-
    /// session registry based on the owner enum.
    async fn report_warning(&self, owner: &SessionOwner, reason: &str) {
        let persona = match owner {
            SessionOwner::Local(name) => self.personas.read().await.get(name).cloned(),
            SessionOwner::Guest(key) => self
                .guest_sessions
                .read()
                .await
                .get(key)
                .map(|g| g.persona.clone()),
        };
        let Some(persona) = persona else { return };
        let strikes = persona.strikes.fetch_add(1, Ordering::Relaxed) + 1;
        *persona.last_warning.write().await = Some(reason.to_string());
        let new_state = if strikes >= 2 {
            PersonaState::Blocked
        } else {
            PersonaState::Suspect
        };
        *persona.state.write().await = new_state;
        match owner {
            SessionOwner::Local(name) => tracing::warn!(
                persona = %name,
                strikes,
                state = ?new_state,
                reason = %reason,
                "local persona warning"
            ),
            SessionOwner::Guest(key) => tracing::warn!(
                peer = %key.peer_id,
                persona = %key.persona_name,
                strikes,
                state = ?new_state,
                reason = %reason,
                "guest session warning"
            ),
        }
    }

    pub async fn click(&self, session_id: &str, selector: &str) -> Result<String, McpError> {
        let session = self.lookup(session_id).await?;
        let _g = session.serial.lock().await;
        session.touch();
        let el = session
            .page
            .find_element(selector)
            .await
            .map_err(|e| invalid(format!("find_element {selector:?}: {e}")))?;
        el.click()
            .await
            .map_err(|e| invalid(format!("click {selector:?}: {e}")))?;
        // Best-effort: a click can trigger navigation; bound the wait
        // at 5s so a same-page click (no navigation) returns promptly.
        let _ =
            tokio::time::timeout(Duration::from_secs(5), session.page.wait_for_navigation()).await;
        session.touch();
        let url = session
            .page
            .url()
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        // If the click navigated to a private host on a restricted
        // session, back out to about:blank and refuse so a peer can't
        // chain a public landing page into a private internal one.
        if session.restrict_to_public && !url.is_empty() && url != "about:blank" {
            if let Err(e) = crate::skills::ssrf::assert_public(&url).await {
                let _ = session.page.goto("about:blank").await;
                return Err(e);
            }
        }
        Ok(url)
    }

    pub async fn type_text(
        &self,
        session_id: &str,
        selector: &str,
        text: &str,
        submit: bool,
    ) -> Result<String, McpError> {
        let session = self.lookup(session_id).await?;
        let _g = session.serial.lock().await;
        session.touch();
        let el = session
            .page
            .find_element(selector)
            .await
            .map_err(|e| invalid(format!("find_element {selector:?}: {e}")))?;
        el.focus()
            .await
            .map_err(|e| invalid(format!("focus {selector:?}: {e}")))?;
        el.type_str(text)
            .await
            .map_err(|e| invalid(format!("type_str {selector:?}: {e}")))?;
        if submit {
            // Press Enter — chromiumoxide doesn't have a one-liner for
            // this on Element, so we eval a tiny scriptlet that
            // dispatches a keypress to the focused element.
            let _ = session
                .page
                .evaluate(
                    "document.activeElement && document.activeElement.form && \
                     document.activeElement.form.requestSubmit && \
                     document.activeElement.form.requestSubmit()",
                )
                .await;
            let _ =
                tokio::time::timeout(Duration::from_secs(15), session.page.wait_for_navigation())
                    .await;
        }
        session.touch();
        let url = session
            .page
            .url()
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        if session.restrict_to_public && !url.is_empty() && url != "about:blank" {
            if let Err(e) = crate::skills::ssrf::assert_public(&url).await {
                let _ = session.page.goto("about:blank").await;
                return Err(e);
            }
        }
        Ok(url)
    }

    /// Wait until at least one element matches `selector`, or `timeout_ms`
    /// elapses. Returns `true` if a match was found, `false` on timeout.
    /// Implementation polls every 100ms — chromiumoxide doesn't expose a
    /// CDP-native `WaitForSelector` helper.
    pub async fn wait(
        &self,
        session_id: &str,
        selector: &str,
        timeout_ms: u64,
    ) -> Result<bool, McpError> {
        let session = self.lookup(session_id).await?;
        let _g = session.serial.lock().await;
        session.touch();
        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            if session.page.find_element(selector).await.is_ok() {
                session.touch();
                return Ok(true);
            }
            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Extract text or an attribute from every element matching `selector`.
    /// When `attr` is `None`, returns each element's `innerText`; otherwise
    /// the attribute's value (missing attributes are returned as empty
    /// strings so the result list aligns with the selector match order).
    pub async fn extract(
        &self,
        session_id: &str,
        selector: &str,
        attr: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, McpError> {
        let session = self.lookup(session_id).await?;
        let _g = session.serial.lock().await;
        session.touch();
        let elements = session
            .page
            .find_elements(selector)
            .await
            .map_err(|e| invalid(format!("find_elements {selector:?}: {e}")))?;
        let mut out: Vec<String> = Vec::with_capacity(elements.len().min(limit));
        for el in elements.into_iter().take(limit) {
            let value = match attr {
                Some(a) => el.attribute(a).await.ok().flatten().unwrap_or_default(),
                None => el.inner_text().await.ok().flatten().unwrap_or_default(),
            };
            out.push(value);
        }
        session.touch();
        Ok(out)
    }

    pub async fn close(&self, session_id: &str) -> Result<(), McpError> {
        let session = {
            let mut table = self.sessions.write().await;
            table
                .remove(session_id)
                .ok_or_else(|| invalid(format!("unknown session_id: {session_id}")))?
        };
        self.session_owner.write().await.remove(session_id);
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
        // Snapshot the session metadata without touching the pages — the
        // dashboard list endpoint enriches with live URL/title via the
        // separate `list_live` call, keeping the cheap path cheap.
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
                    url: None,
                    title: None,
                }
            })
            .collect();
        rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        rows
    }

    /// Same as `list`, but each row also carries the live page URL and
    /// title. Used by the dashboard's snapshot push (one CDP round-trip
    /// per session per 5s tick — cheap for the default cap of 8).
    pub async fn list_live(&self) -> Vec<SessionSummary> {
        let snapshots: Vec<Arc<Session>> = {
            let table = self.sessions.read().await;
            table.values().cloned().collect()
        };
        let now = now_ms();
        let mut rows: Vec<SessionSummary> = Vec::with_capacity(snapshots.len());
        for s in snapshots {
            let last = s.last_used_ms.load(Ordering::Relaxed);
            // Concurrent tools on the same session serialize via
            // s.serial; we *try_lock* so a busy session doesn't stall
            // the dashboard tick. Missing URL/title degrades the row
            // but doesn't break it.
            let (url, title) = match s.serial.try_lock() {
                Ok(_g) => (
                    s.page.url().await.unwrap_or_default(),
                    s.page.get_title().await.unwrap_or_default(),
                ),
                Err(_) => (None, None),
            };
            rows.push(SessionSummary {
                session_id: s.id.clone(),
                created_secs_ago: ((now - s.created_at_ms) / 1000).max(0) as u64,
                idle_secs: ((now - last) / 1000).max(0) as u64,
                url,
                title,
            });
        }
        rows.sort_by(|a, b| a.session_id.cmp(&b.session_id));
        rows
    }

    /// Apply a sparse patch to the runtime config. Same pattern as the
    /// memory / constellation drawers. Values are clamped to safe
    /// ranges. Returns the post-patch state.
    pub async fn apply_runtime_patch(&self, patch: BrowserConfigPatch) -> BrowserSessionConfig {
        let mut cfg = self.cfg.write().await;
        if let Some(v) = patch.idle_timeout_secs {
            cfg.idle_timeout_secs = v.clamp(30, 24 * 3600);
        }
        if let Some(v) = patch.max_concurrent {
            cfg.max_concurrent = v.clamp(1, 64);
        }
        cfg.clone()
    }

    /// Run an arbitrary JS expression in the page and return its result
    /// as a `serde_json::Value`. The expression runs with `awaitPromise:
    /// true` so async work resolves before we return. Use this for the
    /// 1% of cases the granular tools don't cover — keyboard shortcuts,
    /// scroll, mutation observer setup, etc.
    pub async fn eval(
        &self,
        session_id: &str,
        script: &str,
    ) -> Result<serde_json::Value, McpError> {
        let session = self.lookup(session_id).await?;
        if session.restrict_to_public {
            // Raw JS gives the caller a `fetch()` to anywhere — bypasses
            // every URL guard we've put on `navigate`. The conservative
            // policy on delegated sessions is "no eval", which still
            // leaves click / type / extract / wait as the navigation
            // surface. Future work (#130 follow-up): a CDP request
            // interceptor that bans private hosts at the network layer
            // so eval can come back on with safety.
            return Err(invalid(
                "browser_eval is disabled on delegated sessions to prevent SSRF via fetch()"
                    .to_string(),
            ));
        }
        let _g = session.serial.lock().await;
        session.touch();
        let result = session
            .page
            .evaluate(script)
            .await
            .map_err(|e| invalid(format!("evaluate: {e}")))?;
        session.touch();
        Ok(result.into_value().unwrap_or(serde_json::Value::Null))
    }

    /// PNG screenshot of the viewport (or the full scroll height when
    /// `full_page` is `true`). Returned as base64 so the JSON tool
    /// response carries it directly.
    pub async fn screenshot(&self, session_id: &str, full_page: bool) -> Result<String, McpError> {
        use base64::Engine;
        use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
        use chromiumoxide::page::ScreenshotParams;
        let session = self.lookup(session_id).await?;
        let _g = session.serial.lock().await;
        session.touch();
        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(full_page)
            .build();
        let bytes = session
            .page
            .screenshot(params)
            .await
            .map_err(|e| internal(anyhow::anyhow!("screenshot: {e}")))?;
        session.touch();
        Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    /// Build the observation the caller asked for. `None` → empty; the
    /// other modes run the JS scriptlet that walks the DOM picking
    /// interactive elements with their effective ARIA role + name +
    /// stable selector, and/or take a viewport PNG.
    pub async fn observe(
        &self,
        session_id: &str,
        mode: ObserveMode,
    ) -> Result<Observation, McpError> {
        if matches!(mode, ObserveMode::None) {
            return Ok(Observation::default());
        }
        let session = self.lookup(session_id).await?;
        let _g = session.serial.lock().await;
        session.touch();
        let mut obs = Observation::default();
        if matches!(mode, ObserveMode::Tree | ObserveMode::Both) {
            let value = session
                .page
                .evaluate(OBSERVATION_SCRIPT)
                .await
                .ok()
                .and_then(|r| r.into_value::<Vec<TreeNode>>().ok());
            obs.tree = value.or_else(|| Some(Vec::new()));
        }
        if matches!(mode, ObserveMode::Screenshot | ObserveMode::Both) {
            use base64::Engine;
            use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
            use chromiumoxide::page::ScreenshotParams;
            let params = ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .full_page(false)
                .build();
            if let Ok(bytes) = session.page.screenshot(params).await {
                obs.screenshot_b64 = Some(base64::engine::general_purpose::STANDARD.encode(bytes));
            }
        }
        session.touch();
        Ok(obs)
    }

    /// Live URL of the named session. `None` if the session doesn't
    /// exist (was reaped, was never opened). Used by the constellation
    /// delegation path (#128) to enrich the response after a remote
    /// navigate.
    pub async fn session_url(&self, session_id: &str) -> Option<String> {
        let session = self.sessions.read().await.get(session_id).cloned()?;
        session.page.url().await.unwrap_or_default()
    }

    /// Live title of the named session. Counterpart to `session_url`.
    pub async fn session_title(&self, session_id: &str) -> Option<String> {
        let session = self.sessions.read().await.get(session_id).cloned()?;
        session.page.get_title().await.unwrap_or_default()
    }

    // ----------------------------------------------------------------
    // Persona ops — LOCAL warm identities the model owns
    // ----------------------------------------------------------------
    //
    // These act on `self.personas` only. The dashboard table "Your
    // personas" comes from here. `browser_persona_get` is the model-
    // facing tool. Peer-hosted state lives under the `guest_session_*`
    // methods below — they're a separate concept with a separate
    // registry and a separate dashboard table.

    /// Get or create the named LOCAL persona's session and return its
    /// id + state. The model's own `browser_persona_get` tool calls
    /// this. Always unrestricted — the operator opted in to running
    /// tools locally, so SSRF guards don't apply.
    pub async fn persona_get(&self, name: &str) -> Result<(String, PersonaState), McpError> {
        let persona = self.ensure_persona(name).await;
        persona.touch();
        let state = *persona.state.read().await;
        if state == PersonaState::Blocked {
            return Err(invalid(format!(
                "persona {name:?} is blocked — reset from the dashboard before reusing"
            )));
        }
        let mut slot = persona.session_id.write().await;
        if let Some(id) = slot.as_ref() {
            if self.sessions.read().await.contains_key(id) {
                return Ok((id.clone(), state));
            }
        }
        let (id, _, _) = self.open().await?;
        *slot = Some(id.clone());
        self.session_owner
            .write()
            .await
            .insert(id.clone(), SessionOwner::Local(persona.name.clone()));
        Ok((id, state))
    }

    /// Force a fresh session on a LOCAL persona. Disposes the old
    /// session + context and creates a new one in `Healthy` state.
    /// Bound to the dashboard's "reset" button on the personas table.
    pub async fn persona_reset(&self, name: &str) -> Result<String, McpError> {
        let persona = self.ensure_persona(name).await;
        let old = persona.session_id.write().await.take();
        if let Some(id) = old {
            let _ = self.close(&id).await;
        }
        persona.strikes.store(0, Ordering::Relaxed);
        *persona.state.write().await = PersonaState::Healthy;
        *persona.last_warning.write().await = None;
        let (id, _, _) = self.open().await?;
        *persona.session_id.write().await = Some(id.clone());
        self.session_owner
            .write()
            .await
            .insert(id.clone(), SessionOwner::Local(name.to_string()));
        Ok(id)
    }

    /// Snapshot of every LOCAL persona — what the dashboard's "Your
    /// personas" table renders and what `browser_persona_list`
    /// returns. Guest sessions (peer-hosted) come from
    /// `guest_session_list` and stay separate.
    pub async fn persona_list(&self) -> Vec<PersonaSummary> {
        let personas = self.personas.read().await;
        let now = now_ms();
        let mut rows: Vec<PersonaSummary> = Vec::with_capacity(personas.len());
        for p in personas.values() {
            rows.push(summarize_persona(p, &self.sessions, now).await);
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows
    }

    // ----------------------------------------------------------------
    // Guest-session ops — tabs we host on behalf of constellation peers
    // ----------------------------------------------------------------
    //
    // These act on `self.guest_sessions` only. The dashboard table
    // "Hosted for peers" comes from here. Driven exclusively by
    // `/constellation/browser_persona` (inbound) — there is no MCP
    // tool that touches this registry, so the local model never sees
    // guest sessions and can't accidentally drive a peer's session.

    /// Get-or-create the guest session for `(peer_id, name)`. Always
    /// restricted (SSRF guard runs on every navigation). Returns the
    /// session id + current state. A blocked guest session errors;
    /// the requesting peer should retry against a peer with the cap.
    pub async fn guest_session_get(
        &self,
        peer_id: &str,
        name: &str,
    ) -> Result<(String, PersonaState), McpError> {
        let key = GuestKey {
            peer_id: peer_id.to_string(),
            persona_name: name.to_string(),
        };
        let guest = self.ensure_guest_session(&key).await;
        guest.persona.touch();
        let state = *guest.persona.state.read().await;
        if state == PersonaState::Blocked {
            return Err(invalid(format!(
                "guest session for peer={peer_id} persona={name:?} is blocked"
            )));
        }
        let mut slot = guest.persona.session_id.write().await;
        if let Some(id) = slot.as_ref() {
            if self.sessions.read().await.contains_key(id) {
                return Ok((id.clone(), state));
            }
        }
        let (id, _, _) = self.open_restricted().await?;
        *slot = Some(id.clone());
        self.session_owner
            .write()
            .await
            .insert(id.clone(), SessionOwner::Guest(key));
        Ok((id, state))
    }

    /// Tear down every guest session owned by a departing peer. Called
    /// from the constellation peer-removal path when a peer drops out
    /// of our peer table (exceeded MAX_PEER_MISSES). Returns the
    /// number of sessions disposed.
    pub async fn evict_guest_sessions_for_peer(&self, peer_id: &str) -> usize {
        let keys: Vec<GuestKey> = {
            let guests = self.guest_sessions.read().await;
            guests
                .keys()
                .filter(|k| k.peer_id == peer_id)
                .cloned()
                .collect()
        };
        let count = keys.len();
        for key in keys {
            if let Some(guest) = self.guest_sessions.read().await.get(&key).cloned() {
                let sid = guest.persona.session_id.write().await.take();
                if let Some(id) = sid {
                    let _ = self.close(&id).await;
                }
            }
            self.guest_sessions.write().await.remove(&key);
            tracing::info!(
                peer = %key.peer_id,
                persona = %key.persona_name,
                "evicted guest session (peer departed)"
            );
        }
        count
    }

    /// Drop guest sessions that haven't been touched for `idle_secs`
    /// AND whose underlying session is already gone. Silent departures
    /// that didn't trigger the peer-table eviction still get cleaned
    /// up this way.
    pub async fn reap_idle_guest_sessions(&self, idle_secs: u64) -> usize {
        let now = now_ms();
        let cutoff_ms = (idle_secs as i64) * 1000;
        let candidates: Vec<GuestKey> = {
            let guests = self.guest_sessions.read().await;
            let mut out = Vec::new();
            for (k, g) in guests.iter() {
                if now - g.persona.last_touched_ms.load(Ordering::Relaxed) >= cutoff_ms {
                    out.push(k.clone());
                }
            }
            out
        };
        let mut count = 0;
        let sessions_snapshot = self.sessions.read().await.clone();
        for key in candidates {
            let guest = match self.guest_sessions.read().await.get(&key).cloned() {
                Some(g) => g,
                None => continue,
            };
            let sid = guest.persona.session_id.read().await.clone();
            if sid
                .as_ref()
                .is_some_and(|id| sessions_snapshot.contains_key(id))
            {
                continue;
            }
            self.guest_sessions.write().await.remove(&key);
            count += 1;
            tracing::info!(
                peer = %key.peer_id,
                persona = %key.persona_name,
                "reaped idle guest session"
            );
        }
        count
    }

    /// Snapshot of every guest session for the dashboard's "Hosted for
    /// peers" table. Each row carries `peer_id` + `persona_name`
    /// separately so the operator can see who they're hosting for and
    /// which named warm slot it is, without parsing a delegated:* key.
    pub async fn guest_session_list(&self) -> Vec<GuestSessionSummary> {
        let guests = self.guest_sessions.read().await;
        let now = now_ms();
        let mut rows: Vec<GuestSessionSummary> = Vec::with_capacity(guests.len());
        for g in guests.values() {
            let summary = summarize_persona(&g.persona, &self.sessions, now).await;
            rows.push(GuestSessionSummary {
                peer_id: g.key.peer_id.clone(),
                persona_name: g.key.persona_name.clone(),
                state: summary.state,
                session_id: summary.session_id,
                url: summary.url,
                last_warning: summary.last_warning,
                age_secs: summary.age_secs,
            });
        }
        rows.sort_by(|a, b| {
            a.peer_id
                .cmp(&b.peer_id)
                .then(a.persona_name.cmp(&b.persona_name))
        });
        rows
    }

    async fn ensure_guest_session(&self, key: &GuestKey) -> Arc<GuestSession> {
        if let Some(g) = self.guest_sessions.read().await.get(key) {
            return g.clone();
        }
        let mut w = self.guest_sessions.write().await;
        w.entry(key.clone())
            .or_insert_with(|| {
                Arc::new(GuestSession {
                    key: key.clone(),
                    persona: Arc::new(Persona::new(key.persona_name.clone())),
                })
            })
            .clone()
    }

    async fn ensure_persona(&self, name: &str) -> Arc<Persona> {
        if let Some(p) = self.personas.read().await.get(name) {
            return p.clone();
        }
        let mut w = self.personas.write().await;
        w.entry(name.to_string())
            .or_insert_with(|| Arc::new(Persona::new(name.to_string())))
            .clone()
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

/// Shared helper: build a `PersonaSummary` from a Persona + the live
/// session table. Used by both `persona_list` (local) and
/// `guest_session_list` (peer-hosted) since the on-the-wire shape of
/// each row's session info is identical — only the row's extra
/// metadata differs.
async fn summarize_persona(
    persona: &Persona,
    sessions: &RwLock<HashMap<String, Arc<Session>>>,
    now: i64,
) -> PersonaSummary {
    let session_id = persona.session_id.read().await.clone();
    let state = *persona.state.read().await;
    let url = if let Some(sid) = &session_id {
        let table = sessions.read().await;
        if let Some(sess) = table.get(sid) {
            sess.page.url().await.unwrap_or_default()
        } else {
            None
        }
    } else {
        None
    };
    PersonaSummary {
        name: persona.name.clone(),
        state,
        session_id,
        url,
        last_warning: persona.last_warning.read().await.clone(),
        age_secs: ((now - persona.created_at_ms) / 1000).max(0) as u64,
    }
}

/// Listing row for `browser_list` and the dashboard. `url`+`title` are
/// `None` when populated through the cheap `list` path; the dashboard
/// fetches them via `list_live` which adds a CDP round-trip per row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub created_secs_ago: u64,
    pub idle_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// Sparse patch the dashboard sends to `/api/settings/browser`. Same
/// shape as the memory / constellation patches.
#[derive(Debug, Default, Deserialize)]
pub struct BrowserConfigPatch {
    pub idle_timeout_secs: Option<u64>,
    pub max_concurrent: Option<usize>,
}

// ---------------------------------------------------------------------------
// Observation scriptlet
// ---------------------------------------------------------------------------
//
// Returns a JSON array of {role, name, selector, value?} for every
// interactive element in the viewport (or near it). Runs entirely in
// the page — no CDP calls during the walk — so it's cheap. Selector
// strategy: id > [data-testid] > a path of nth-of-type fragments back
// to the nearest ancestor with a stable id. Cap at 150 nodes so a
// pathological page can't blow the response size.
const OBSERVATION_SCRIPT: &str = r#"
(() => {
  const SELECTORS = [
    'a[href]', 'button', 'input', 'select', 'textarea',
    '[role=button]', '[role=link]', '[role=textbox]',
    '[role=checkbox]', '[role=radio]', '[role=menuitem]',
    '[role=tab]', '[role=combobox]', '[contenteditable=true]'
  ];
  const els = Array.from(document.querySelectorAll(SELECTORS.join(',')));
  const visible = (el) => {
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const cs = window.getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none') return false;
    return true;
  };
  const trim = (s, n) => {
    const t = (s || '').replace(/\s+/g, ' ').trim();
    return t.length > n ? t.slice(0, n - 1) + '…' : t;
  };
  const selectorFor = (el) => {
    if (el.id) return '#' + CSS.escape(el.id);
    const tid = el.getAttribute('data-testid');
    if (tid) return '[data-testid="' + CSS.escape(tid).replace(/"/g, '\\"') + '"]';
    const parts = [];
    let cur = el;
    while (cur && cur.nodeType === 1 && parts.length < 6) {
      if (cur.id) { parts.unshift('#' + CSS.escape(cur.id)); break; }
      let part = cur.tagName.toLowerCase();
      const parent = cur.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children).filter(c => c.tagName === cur.tagName);
        if (siblings.length > 1) {
          const idx = siblings.indexOf(cur) + 1;
          part += ':nth-of-type(' + idx + ')';
        }
      }
      parts.unshift(part);
      cur = cur.parentElement;
    }
    return parts.join(' > ');
  };
  const roleOf = (el) => {
    const r = el.getAttribute('role');
    if (r) return r;
    const t = el.tagName.toLowerCase();
    if (t === 'a') return 'link';
    if (t === 'button') return 'button';
    if (t === 'input') {
      const it = (el.getAttribute('type') || 'text').toLowerCase();
      if (it === 'checkbox') return 'checkbox';
      if (it === 'radio') return 'radio';
      if (it === 'submit' || it === 'button') return 'button';
      return 'textbox';
    }
    if (t === 'textarea') return 'textbox';
    if (t === 'select') return 'combobox';
    return t;
  };
  const nameOf = (el) => {
    const al = el.getAttribute('aria-label');
    if (al) return trim(al, 120);
    const txt = el.innerText || el.textContent || '';
    if (txt) return trim(txt, 120);
    const ph = el.getAttribute('placeholder');
    if (ph) return trim(ph, 120);
    const tl = el.getAttribute('title');
    if (tl) return trim(tl, 120);
    return '';
  };
  const out = [];
  for (const el of els) {
    if (!visible(el)) continue;
    const row = {
      role: roleOf(el),
      name: nameOf(el),
      selector: selectorFor(el)
    };
    const tag = el.tagName.toLowerCase();
    if (tag === 'input' || tag === 'textarea' || tag === 'select') {
      const v = el.value;
      if (v) row.value = trim(v, 120);
    }
    out.push(row);
    if (out.length >= 150) break;
  }
  return out;
})()
"#;

// ---------------------------------------------------------------------------
// Reaper
// ---------------------------------------------------------------------------

fn spawn_reaper(manager: Arc<BrowserSessionManager>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            let timeout_secs = manager.config().await.idle_timeout_secs;
            let timeout = timeout_secs as i64 * 1000;
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
            // Guest-session cleanup: a guest session whose owner walked
            // away leaves a tiny bookkeeping struct sitting in memory
            // even after the underlying session is reaped. Give it
            // twice the session idle window before dropping the entry.
            // Local personas are NEVER auto-reaped — the operator owns
            // their lifecycle.
            let _ = manager.reap_idle_guest_sessions(timeout_secs * 2).await;
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

/// Non-initializing accessor used by the dashboard snapshot path —
/// returns `None` when no browser tool has been called yet so the
/// snapshot stays cheap when the model isn't using the browser.
pub fn manager_if_init() -> Option<Arc<BrowserSessionManager>> {
    MANAGER.get().cloned()
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
            Ok(json(serde_json::json!({
                "session_id": id,
                "url": url,
                "tip": "pass session_id to browser_navigate / browser_click / etc. Use `observe: \"tree\"` on those tools to get a list of interactive elements to act on."
            })))
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
    /// What to return after the navigation settles: `"none"` (just the
    /// resulting URL + title), `"tree"` (interactive elements with
    /// selectors), `"screenshot"`, or `"both"`. Default `"none"`.
    #[serde(default)]
    observe: ObserveMode,
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
            let obs = mgr.observe(&args.session_id, args.observe).await?;
            Ok(json(serde_json::json!({
                "url": url,
                "title": title,
                "observation": obs,
            })))
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

// ---------------------------------------------------------------------------
// Interaction tools
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ClickArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
    /// CSS selector for the element to click — e.g. `button[type=submit]`,
    /// `a[href*="/sign-in"]`, `#login`, `[role=button][aria-label="Search"]`.
    /// Falls back to no-match error if the element isn't on the page.
    selector: String,
    /// What to return after the click: `"none"`, `"tree"`, `"screenshot"`,
    /// or `"both"`. Default `"none"`.
    #[serde(default)]
    observe: ObserveMode,
}

pub struct BrowserClick;
impl Skill for BrowserClick {
    fn name(&self) -> &'static str {
        "browser_click"
    }
    fn description(&self) -> &'static str {
        "Click the first element matching `selector` in the named session. If the click triggers \
         navigation, waits up to 5s for it to settle before returning. Returns the resulting URL. \
         If the element isn't on the page yet, use `browser_wait` first so the model can react to \
         a load-failure rather than the click silently no-op'ing."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ClickArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ClickArgs>()?;
            let mgr = manager().await;
            let url = mgr.click(&args.session_id, &args.selector).await?;
            let obs = mgr.observe(&args.session_id, args.observe).await?;
            Ok(json(serde_json::json!({ "url": url, "observation": obs })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TypeArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
    /// CSS selector for the input/textarea to focus and type into.
    selector: String,
    /// Text to type. Use literal characters; modifier keys aren't supported
    /// here (use `browser_eval` for a keyboard event if you need them).
    text: String,
    /// When `true`, attempts to submit the enclosing form after typing
    /// (calls `form.requestSubmit()`). Use for search boxes whose submit
    /// is the Enter key. Default `false`.
    #[serde(default)]
    submit: bool,
    /// Observation mode for the post-action snapshot. Default `"none"`.
    #[serde(default)]
    observe: ObserveMode,
}

pub struct BrowserType;
impl Skill for BrowserType {
    fn name(&self) -> &'static str {
        "browser_type"
    }
    fn description(&self) -> &'static str {
        "Focus the element matching `selector` and type `text` into it. With `submit: true`, \
         submits the enclosing form via `form.requestSubmit()` and waits up to 15s for navigation \
         to settle (so a search box round-trip is one call). Returns the resulting URL."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TypeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<TypeArgs>()?;
            let mgr = manager().await;
            let url = mgr
                .type_text(&args.session_id, &args.selector, &args.text, args.submit)
                .await?;
            let obs = mgr.observe(&args.session_id, args.observe).await?;
            Ok(json(serde_json::json!({ "url": url, "observation": obs })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaitArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
    /// CSS selector to wait for. The wait succeeds as soon as the first
    /// match exists in the DOM (matches don't need to be visible — pair
    /// with a selector that filters on visibility if that matters).
    selector: String,
    /// Max time to wait, in milliseconds. Clamped to [50, 60000].
    /// Default 5000.
    #[serde(default)]
    timeout_ms: Option<u64>,
}

pub struct BrowserWait;
impl Skill for BrowserWait {
    fn name(&self) -> &'static str {
        "browser_wait"
    }
    fn description(&self) -> &'static str {
        "Poll until at least one element matches `selector`, or `timeout_ms` elapses (default \
         5000, max 60000). Returns `{matched: true}` on success and `{matched: false}` on \
         timeout — the model should branch on this rather than treating timeout as an error."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WaitArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<WaitArgs>()?;
            let timeout_ms = args.timeout_ms.unwrap_or(5000).clamp(50, 60_000);
            let mgr = manager().await;
            let matched = mgr
                .wait(&args.session_id, &args.selector, timeout_ms)
                .await?;
            Ok(json(serde_json::json!({ "matched": matched })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExtractArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
    /// CSS selector. Every match contributes one entry to `values`.
    selector: String,
    /// Attribute name to extract. When omitted, returns `innerText` for
    /// each match instead. Common attrs: `href`, `src`, `value`,
    /// `aria-label`, `data-*`.
    #[serde(default)]
    attr: Option<String>,
    /// Maximum results to return — extra matches are dropped. Clamped to
    /// [1, 500]. Default 50.
    #[serde(default)]
    limit: Option<usize>,
}

pub struct BrowserExtract;
impl Skill for BrowserExtract {
    fn name(&self) -> &'static str {
        "browser_extract"
    }
    fn description(&self) -> &'static str {
        "Read text or attribute values from every element matching `selector`. With no `attr`, \
         returns each element's `innerText` (visible text content). With `attr`, returns each \
         element's value for that attribute (missing attributes yield empty strings, so the \
         result list aligns with selector order). Capped at `limit` results (default 50, max \
         500). Use this to scrape a page after navigation — search results, link hrefs, table \
         cells, etc."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ExtractArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ExtractArgs>()?;
            let limit = args.limit.unwrap_or(50).clamp(1, 500);
            let mgr = manager().await;
            let values = mgr
                .extract(
                    &args.session_id,
                    &args.selector,
                    args.attr.as_deref(),
                    limit,
                )
                .await?;
            Ok(json(serde_json::json!({ "values": values })))
        })
    }
}

// ---------------------------------------------------------------------------
// Observation tools
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvalArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
    /// JS expression to evaluate in the page. Wrap multi-statement
    /// scripts in an IIFE (`(() => { ... })()`). The result is
    /// serialized to JSON — Promises are awaited (`awaitPromise: true`),
    /// non-JSON values (DOM nodes, functions) return `null`.
    script: String,
}

pub struct BrowserEval;
impl Skill for BrowserEval {
    fn name(&self) -> &'static str {
        "browser_eval"
    }
    fn description(&self) -> &'static str {
        "Run an arbitrary JS expression in the page and return its result as JSON. Use this for \
         the 1% of cases the granular tools don't cover — scrolling, keyboard shortcuts, mutation \
         observer setup, reading window.* state. Promises are awaited. Wrap multi-statement \
         scripts in an IIFE: `(() => { ...; return value; })()`. Returns `{result: <json>}`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EvalArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<EvalArgs>()?;
            let mgr = manager().await;
            let result = mgr.eval(&args.session_id, &args.script).await?;
            Ok(json(serde_json::json!({ "result": result })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScreenshotArgs {
    /// Session id returned by `browser_open`.
    session_id: String,
    /// When `true`, the screenshot captures the full scroll height of
    /// the page (not just the viewport). Default `false`.
    #[serde(default)]
    full_page: bool,
}

pub struct BrowserScreenshot;
impl Skill for BrowserScreenshot {
    fn name(&self) -> &'static str {
        "browser_screenshot"
    }
    fn description(&self) -> &'static str {
        "Take a PNG screenshot of the session's current page. With `full_page: true`, the image \
         covers the entire scroll height; otherwise just the viewport. Returned as base64 in \
         `{png_b64}`. The other tools' `observe: \"screenshot\"`/`\"both\"` flag is usually a \
         better fit — use this tool when you just want the image without driving an action."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ScreenshotArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ScreenshotArgs>()?;
            let mgr = manager().await;
            let png_b64 = mgr.screenshot(&args.session_id, args.full_page).await?;
            Ok(json(serde_json::json!({ "png_b64": png_b64 })))
        })
    }
}

// ---------------------------------------------------------------------------
// Persona tools (#127)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PersonaGetArgs {
    /// Persona name — by convention, the bare hostname or vendor key
    /// (e.g. `"google"`, `"stackoverflow"`, `"github"`). Routing all
    /// queries against one site through one named persona accumulates a
    /// warm session (cookies, solved-CAPTCHA tokens) that defeats most
    /// per-IP rate limits.
    name: String,
}

pub struct BrowserPersonaGet;
impl Skill for BrowserPersonaGet {
    fn name(&self) -> &'static str {
        "browser_persona_get"
    }
    fn description(&self) -> &'static str {
        "Return a session_id for the named long-lived persona, creating the persona if it doesn't exist. \
         Subsequent `browser_navigate` / `browser_click` / etc. on that session reuse the persona's \
         warm state. Returns `{session_id, state}`. A persona in `\"blocked\"` state (CAPTCHA stuck / \
         403 challenge) returns an error — the operator must reset it from the dashboard. A persona \
         in `\"suspect\"` state still works but the model should consider backing off / using a \
         different provider for a few minutes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PersonaGetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<PersonaGetArgs>()?;
            let mgr = manager().await;
            let (session_id, state) = mgr.persona_get(&args.name).await?;
            Ok(json(serde_json::json!({
                "session_id": session_id,
                "state": state,
            })))
        })
    }
}

pub struct BrowserPersonaList;
impl Skill for BrowserPersonaList {
    fn name(&self) -> &'static str {
        "browser_persona_list"
    }
    fn description(&self) -> &'static str {
        "List every named browser persona with its current state (`healthy` / `suspect` / `blocked`), \
         the last warning that flipped it out of healthy (if any), and the underlying session id. \
         Personas survive the model's individual flows so this is the right tool to check before a \
         long scrape."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let _ = ctx.parse::<NoArgs>()?;
            let mgr = manager().await;
            let personas = mgr.persona_list().await;
            Ok(json(serde_json::json!({ "personas": personas })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PersonaResetArgs {
    /// Persona name to reset. Disposes the current session + context and
    /// starts a fresh one. State returns to `healthy`. Use this when
    /// a persona is `blocked` (CAPTCHA didn't clear, account got flagged)
    /// or to deliberately rotate the warm state.
    name: String,
}

pub struct BrowserPersonaReset;
impl Skill for BrowserPersonaReset {
    fn name(&self) -> &'static str {
        "browser_persona_reset"
    }
    fn description(&self) -> &'static str {
        "Force a fresh session on the named persona: dispose the current tab + context and spin up a \
         new one. State returns to `healthy`. Use this when a persona is `blocked` or when you want \
         to deliberately rotate its warm state."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PersonaResetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<PersonaResetArgs>()?;
            let mgr = manager().await;
            let session_id = mgr.persona_reset(&args.name).await?;
            Ok(json(serde_json::json!({
                "session_id": session_id,
                "state": "healthy",
            })))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PersonaDelegateArgs {
    /// Persona name on the REMOTE node. Each node maintains its own
    /// named personas; this isn't transporting our session, it's asking
    /// a peer to run the navigate on its OWN persona.
    persona_name: String,
    /// URL to navigate to on the peer's persona. Subject to the peer's
    /// SSRF guard, which refuses any URL that resolves to its local
    /// network.
    url: String,
}

pub struct BrowserPersonaDelegate;
impl Skill for BrowserPersonaDelegate {
    fn name(&self) -> &'static str {
        "browser_persona_delegate"
    }
    fn description(&self) -> &'static str {
        "Ask a constellation peer (a node that opted in with \
         `[network.capabilities].browser = true`) to navigate ITS named persona to a URL and return \
         the compact observation tree. Use this when our local persona is in `blocked` state (rate \
         limited / CAPTCHA stuck) — the peer has a different IP and its own warm session, so the \
         same query often succeeds where ours just bounced. The peer's SSRF guard refuses any \
         URL that resolves to ITS local network, so this can't be used to enumerate the peer's \
         LAN. Returns `{url, title, tree}`. Sessions themselves never transport: each node uses \
         its own persona."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PersonaDelegateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PersonaDelegateArgs>()?;
            let constellation = server.registry.constellation().ok_or_else(|| {
                invalid(
                    "constellation is disabled ([network].enabled = false) so there is no \
                     peer to delegate to"
                        .to_string(),
                )
            })?;
            let req = crate::constellation::BrowserPersonaReq {
                persona_name: args.persona_name,
                url: args.url,
            };
            let resp = constellation
                .delegate_browser_persona(req)
                .await
                .map_err(invalid)?;
            Ok(json(serde_json::json!({
                "url": resp.url,
                "title": resp.title,
                "tree": resp.tree,
            })))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(BrowserOpen),
        Box::new(BrowserNavigate),
        Box::new(BrowserClose),
        Box::new(BrowserList),
        Box::new(BrowserClick),
        Box::new(BrowserType),
        Box::new(BrowserWait),
        Box::new(BrowserExtract),
        Box::new(BrowserEval),
        Box::new(BrowserScreenshot),
        Box::new(BrowserPersonaGet),
        Box::new(BrowserPersonaList),
        Box::new(BrowserPersonaReset),
        Box::new(BrowserPersonaDelegate),
    ]
}
