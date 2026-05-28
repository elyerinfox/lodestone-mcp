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

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result};

/// Gating data: the whole tool is gated by `[git].enabled`. Destructive
/// *subcommands* are checked at call time (not a separate tool), so no entries here.
pub const TOOL_NAMES: &[&str] = &["git_run"];
pub const DESTRUCTIVE_NAMES: &[&str] = &[];

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
}

pub struct GitRun;
impl Skill for GitRun {
    fn name(&self) -> &'static str {
        "git_run"
    }
    fn description(&self) -> &'static str {
        "Run a git command in a repository (runs the local `git` binary; no shell). Pass the args \
        without the leading `git`, e.g. `status -sb`, `log --oneline -10`, `diff`, `commit -m \
        \"msg\"`. Destructive subcommands (push/reset/clean/rebase/…) need [git].allow_destructive. \
        Returns exit code + stdout/stderr."
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
            if DESTRUCTIVE_SUBCMDS.contains(&subcmd) && !cfg.allow_destructive {
                return Err(invalid(format!(
                    "git '{subcmd}' is destructive; set [git].allow_destructive to allow it"
                )));
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
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(GitRun)]
}
