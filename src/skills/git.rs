//! Git CLI skill — `git_run`. Runs the local `git` binary directly (no shell, so
//! arguments aren't re-interpreted), against a repository working directory.
//!
//! On by default (`[git].enabled`); destructive subcommands (push, reset, clean,
//! rebase, filter-branch, gc, prune, reflog) are blocked unless
//! `[git].allow_destructive`. Requires `git` on PATH.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use tokio::process::Command;

use crate::skills::guard::Decision;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result};

/// Subcommands blocked unless `[git].allow_destructive` is set.
const DESTRUCTIVE_SUBCMDS: &[&str] = &[
    "push",
    "reset",
    "clean",
    "rebase",
    "filter-branch",
    "filter-repo",
    "gc",
    "prune",
    "reflog",
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GitRunArgs {
    /// The git arguments, e.g. `status -sb`, `log --oneline -10`, `commit -m "msg"`.
    /// (Do not include the leading `git`.)
    args: String,
    /// Repository working directory. Omit to use `[git].repo` or the server's CWD.
    #[serde(default)]
    repo: Option<String>,
    /// One-time token from a prior call's confirmation prompt (only needed for a
    /// destructive subcommand). Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this git subcommand for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct GitRun;
impl Skill for GitRun {
    fn name(&self) -> &'static str {
        "git_run"
    }
    fn description(&self) -> &'static str {
        "Run a git command in a repository (runs the local `git` binary; no shell). Pass the args \
        without the leading `git`, e.g. `status -sb`, `log --oneline -10`, `diff`, `commit -m \
        \"msg\"`. Destructive subcommands (push/reset/clean/rebase/…) return a confirmation token on \
        the first call and do nothing — call again with confirm=<token> to proceed (or confirm + \
        trust=true to allow that subcommand for the session). Returns exit code + stdout/stderr."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GitRunArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<GitRunArgs>()?;
            let cfg = &server.git;
            let tokens = shell_words::split(args.args.trim())
                .map_err(|e| invalid(format!("could not parse git args: {e}")))?;
            if tokens.is_empty() {
                return Err(invalid("no git arguments given"));
            }
            // First non-flag token is the subcommand.
            let subcmd = tokens
                .iter()
                .find(|t| !t.starts_with('-'))
                .map(|s| s.as_str())
                .unwrap_or("");
            if DESTRUCTIVE_SUBCMDS.contains(&subcmd) {
                let summary = format!("git {}", args.args.trim());
                if let Decision::Challenge(msg) = server.guard.check(
                    &format!("git:{subcmd}"),
                    "git_run",
                    cfg.allow_destructive,
                    &summary,
                    args.confirm.as_deref(),
                    args.trust.unwrap_or(false),
                ) {
                    return Ok(text_result(msg));
                }
            }

            let workdir = args
                .repo
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| Some(cfg.repo.trim()).filter(|s| !s.is_empty()));

            let mut cmd = Command::new("git");
            cmd.args(&tokens);
            if let Some(dir) = workdir {
                cmd.current_dir(dir);
            }
            cmd.stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            let child = cmd.spawn().map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    invalid("the `git` binary was not found on PATH — install Git to use git_run")
                } else {
                    invalid(format!("could not run git: {e}"))
                }
            })?;
            let secs = cfg.timeout_secs.clamp(1, 600);
            let output =
                match tokio::time::timeout(Duration::from_secs(secs), child.wait_with_output())
                    .await
                {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => return Err(internal(e.into())),
                    Err(_) => {
                        return Ok(text_result(format!(
                            "git {}\n(timed out after {secs}s and was killed)",
                            args.args.trim()
                        )))
                    }
                };

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string());
            let mut out = format!("git {}\n(exit {code})\n", args.args.trim());
            if !stdout.trim().is_empty() {
                out.push_str(&format!("\n{stdout}"));
            }
            if !stderr.trim().is_empty() {
                out.push_str(&format!("\n--- stderr ---\n{stderr}"));
            }
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Short status",
                args: r#"{"args": "status -sb"}"#,
                note: Some("Read-only; no confirmation needed."),
            },
            SkillExample {
                title: "Recent log",
                args: r#"{"args": "log --oneline -10"}"#,
                note: None,
            },
            SkillExample {
                title: "Diff of working tree",
                args: r#"{"args": "diff", "repo": "."}"#,
                note: Some("Override `repo` to point at a different working copy."),
            },
            SkillExample {
                title: "Destructive subcommand (push) — second call",
                args: r#"{"args": "push origin main", "confirm": "<token-from-prior-call>"}"#,
                note: Some("First call without `confirm` returns a token; second call runs it. Add `trust: true` to whitelist this subcommand for the session."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Inspect a repo's state (status / log / diff / show / branch / remote).",
            "Stage and commit code changes from inside an LLM-driven workflow.",
            "Run a mutating subcommand (push / reset / rebase) with explicit confirmation.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "args",
            min: Some(1),
            max: None,
        }]
    }
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "git"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Run `git` against a local repository for inspection (status / log / diff / show / \
         branch / remote) and, with confirmation, mutating subcommands. Paths confined to \
         the configured roots. Requires the `git` binary on `$PATH`."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::{binary_on_path, SkillCapability};
        if binary_on_path("git") {
            SkillCapability::Ready
        } else {
            SkillCapability::unavailable(
                "no `git` binary on PATH",
                "install git or extend the container image",
            )
        }
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(GitRun)]
}
