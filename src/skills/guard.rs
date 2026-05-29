//! Confirmation guard for destructive actions — a **client-agnostic** alternative
//! to MCP elicitation (which some clients, e.g. LM Studio, don't support).
//!
//! A destructive tool calls [`Guard::check`] *before* acting. Unless the action is
//! pre-authorized (`[family].allow_destructive`) or already trusted this session,
//! the first call performs **nothing**: it returns a one-time token describing
//! exactly what will happen. The tool must be called again with that `confirm`
//! token to actually run — so a destructive op can never be executed in a single,
//! un-surfaced step (and the client's own per-call approval UI sits on the
//! executing call). Passing `trust: true` alongside the token also whitelists the
//! tool for the rest of the process (in-memory; cleared on restart).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How long a confirmation token stays valid.
const TOKEN_TTL: Duration = Duration::from_secs(300);

/// Outcome of [`Guard::check`].
pub enum Decision {
    /// Authorized — run the action.
    Proceed,
    /// Not authorized yet — return this message to the caller; take no action.
    Challenge(String),
}

/// Session-scoped confirmation state: trusted tools + outstanding tokens. Cheap to
/// clone (shared `Arc`), so it lives on [`crate::Lodestone`].
#[derive(Clone, Default)]
pub struct Guard {
    inner: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    /// Tool names trusted for the rest of the session (skip the prompt).
    trusted: HashSet<String>,
    /// Outstanding challenge tokens → the action they authorize.
    pending: HashMap<String, Pending>,
}

struct Pending {
    key: String,
    expires: Instant,
}

impl Guard {
    /// Decide whether an action may run now.
    ///
    /// * `key` — the trust/binding key for *this kind of action* (usually the tool
    ///   name; finer-grained where one tool covers several actions, e.g.
    ///   `"git:push"`). Trust and token-binding are keyed on this.
    /// * `tool` — the MCP tool name to re-invoke with the token (shown in the prompt).
    /// * `pre_authorized` — the family's `allow_destructive` flag (skip the prompt).
    /// * `summary` — a human description of the exact action ("remove container web").
    /// * `confirm` / `trust` — taken from the tool's own arguments.
    pub fn check(
        &self,
        key: &str,
        tool: &str,
        pre_authorized: bool,
        summary: &str,
        confirm: Option<&str>,
        trust: bool,
    ) -> Decision {
        if pre_authorized {
            return Decision::Proceed;
        }
        let mut st = self.inner.lock().unwrap();
        if st.trusted.contains(key) {
            return Decision::Proceed;
        }
        if let Some(tok) = confirm.map(str::trim).filter(|s| !s.is_empty()) {
            if let Some(p) = st.pending.remove(tok) {
                if p.key == key && p.expires > Instant::now() {
                    if trust {
                        st.trusted.insert(key.to_string());
                    }
                    return Decision::Proceed;
                }
            }
            // Unknown / expired / mismatched token: fall through to a fresh challenge.
        }
        st.prune();
        let token = new_token();
        st.pending.insert(
            token.clone(),
            Pending {
                key: key.to_string(),
                expires: Instant::now() + TOKEN_TTL,
            },
        );
        Decision::Challenge(challenge_message(tool, summary, &token))
    }
}

impl State {
    fn prune(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, p| p.expires > now);
    }
}

fn challenge_message(tool: &str, summary: &str, token: &str) -> String {
    format!(
        "Destructive action NOT performed: {summary}.\n\n\
         This needs confirmation first. Ask the user whether to proceed, then call `{tool}` again with:\n\
         \u{2022} confirm = \"{token}\"  — perform it this once\n\
         \u{2022} confirm = \"{token}\", trust = true  — perform it and stop asking for `{tool}` for the rest of this session\n\n\
         To cancel, simply do not call again. The token expires in 5 minutes. \
         (To skip these prompts entirely, set this family's allow_destructive in config.)"
    )
}

/// A short, unguessable single-use token. Not cryptographic — its only job is to
/// be unknowable until the first call returns it, forcing a deliberate second call.
fn new_token() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in [n, nanos, pid] {
        h ^= b;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let h2 = h.rotate_left(32) ^ nanos.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    format!("{h:016x}{:08x}", h2 as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pull the `confirm` token out of a challenge message.
    fn token_of(d: Decision) -> String {
        match d {
            Decision::Challenge(msg) => msg
                .split("confirm = \"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap()
                .to_string(),
            Decision::Proceed => panic!("expected a challenge"),
        }
    }

    #[test]
    fn pre_authorized_proceeds_without_token() {
        let g = Guard::default();
        assert!(matches!(
            g.check("fs_delete", "fs_delete", true, "delete x", None, false),
            Decision::Proceed
        ));
    }

    #[test]
    fn challenge_then_confirm_roundtrip() {
        let g = Guard::default();
        let token = token_of(g.check("fs_delete", "fs_delete", false, "delete x", None, false));
        // Correct token proceeds; the token is single-use.
        assert!(matches!(
            g.check(
                "fs_delete",
                "fs_delete",
                false,
                "delete x",
                Some(&token),
                false
            ),
            Decision::Proceed
        ));
        assert!(matches!(
            g.check(
                "fs_delete",
                "fs_delete",
                false,
                "delete x",
                Some(&token),
                false
            ),
            Decision::Challenge(_)
        ));
    }

    #[test]
    fn trust_skips_future_prompts() {
        let g = Guard::default();
        let token = token_of(g.check("k8s_delete", "k8s_delete", false, "delete pod", None, false));
        assert!(matches!(
            g.check(
                "k8s_delete",
                "k8s_delete",
                false,
                "delete pod",
                Some(&token),
                true
            ),
            Decision::Proceed
        ));
        // Now trusted: a later call needs no token.
        assert!(matches!(
            g.check(
                "k8s_delete",
                "k8s_delete",
                false,
                "delete other",
                None,
                false
            ),
            Decision::Proceed
        ));
    }

    #[test]
    fn token_is_bound_to_its_key() {
        let g = Guard::default();
        // A token minted for `git:push` must not authorize `git:reset`.
        let token = token_of(g.check("git:push", "git_run", false, "git push", None, false));
        assert!(matches!(
            g.check(
                "git:reset",
                "git_run",
                false,
                "git reset",
                Some(&token),
                false
            ),
            Decision::Challenge(_)
        ));
    }
}
