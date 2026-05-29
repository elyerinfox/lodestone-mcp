//! Shell-execution skill — `shell_run`. **Arbitrary code execution: the most
//! dangerous capability, OFF by default.** Gated by `[shell].enabled`.
//!
//! Two modes:
//! * **Allowlist** (default): the command's first token (by basename) must be in
//!   `[shell].allow`; the program is then executed **directly, without a shell**,
//!   so `;`, `|`, `$(…)` etc. are inert literals — the allowlist is a real bound.
//! * **Unrestricted** (`[shell].allow_unrestricted`): the whole command runs via
//!   the system shell (`sh -c` / `cmd /C`) — full power, full risk.
//!
//! Each run has a timeout (`kill_on_drop`) and a working directory.

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

/// Gating data (consumed by `skills::disabled_by_config`). The whole tool is
/// gated by `[shell].enabled`; the allowlist/unrestricted policy is enforced at
/// call time.
pub const TOOL_NAMES: &[&str] = &["shell_run"];

/// Program name to match against the allowlist: the first token's basename (split
/// on `/` and `\` on every platform), with a Windows-style executable suffix
/// stripped.
fn program_base(p: &str) -> String {
    let base = p.rsplit(['/', '\\']).next().unwrap_or(p);
    let lower = base.to_ascii_lowercase();
    for ext in [".exe", ".bat", ".cmd", ".com", ".ps1"] {
        if let Some(stripped) = lower.strip_suffix(ext) {
            return base[..stripped.len()].to_string();
        }
    }
    base.to_string()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ShellRunArgs {
    /// The command line to run. In allowlist mode only the first program runs
    /// (no shell), so shell operators are literal; in unrestricted mode the whole
    /// line is interpreted by the system shell.
    command: String,
    /// Working directory. Omit to use `[shell].workdir` or the server's CWD.
    #[serde(default)]
    workdir: Option<String>,
    /// Timeout in seconds (the process is killed when it elapses). Omit for the
    /// configured default; capped at 600.
    #[serde(default)]
    timeout_secs: Option<u32>,
}

pub struct ShellRun;
impl Skill for ShellRun {
    fn name(&self) -> &'static str {
        "shell_run"
    }
    fn description(&self) -> &'static str {
        "Run a shell command on this machine (gated by [shell]; off by default). In allowlist mode \
        only programs in [shell].allow run, executed directly without a shell; in unrestricted mode \
        the whole command runs via the system shell. Returns exit code + stdout/stderr. Powerful \
        and dangerous — runs with the server's privileges."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ShellRunArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ShellRunArgs>()?;
            let cfg = &server.shell;
            let command = args.command.trim().to_string();
            if command.is_empty() {
                return Err(invalid("empty command"));
            }
            let secs = args
                .timeout_secs
                .map(|n| n as u64)
                .unwrap_or(cfg.timeout_secs)
                .clamp(1, 600);

            // Build the command per the policy. `program_label` names the binary
            // that must exist, for a clear "not found" error.
            let program_label;
            let mut cmd = if cfg.allow_unrestricted {
                let mut c = if cfg!(windows) {
                    program_label = "cmd".to_string();
                    let mut c = Command::new("cmd");
                    c.arg("/C").arg(&command);
                    c
                } else {
                    program_label = "sh".to_string();
                    let mut c = Command::new("sh");
                    c.arg("-c").arg(&command);
                    c
                };
                c.kill_on_drop(true);
                c
            } else {
                let tokens = shell_words::split(&command)
                    .map_err(|e| invalid(format!("could not parse command: {e}")))?;
                let program = tokens.first().ok_or_else(|| invalid("empty command"))?;
                let base = program_base(program);
                let allowed = cfg
                    .allow
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case(&base) || a.eq_ignore_ascii_case(program));
                if !allowed {
                    let list = if cfg.allow.is_empty() {
                        "none".to_string()
                    } else {
                        cfg.allow.join(", ")
                    };
                    return Err(invalid(format!(
                        "'{base}' is not in [shell].allow (allowed: {list}; set [shell].allow_unrestricted to run anything)"
                    )));
                }
                program_label = program.clone();
                let mut c = Command::new(program);
                c.args(&tokens[1..]);
                c.kill_on_drop(true);
                c
            };

            // Working directory: per-call override, else config, else process CWD.
            let workdir = args
                .workdir
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| Some(cfg.workdir.trim()).filter(|s| !s.is_empty()));
            if let Some(dir) = workdir {
                cmd.current_dir(dir);
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

            let child = cmd.spawn().map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    invalid(format!(
                        "'{program_label}' was not found on PATH (is it installed?)"
                    ))
                } else {
                    invalid(format!("could not start '{program_label}': {e}"))
                }
            })?;
            let output =
                match tokio::time::timeout(Duration::from_secs(secs), child.wait_with_output())
                    .await
                {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => return Err(internal(e.into())),
                    Err(_) => {
                        return Ok(text_result(format!(
                            "$ {command}\n(timed out after {secs}s and was killed)"
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
            let mut out = format!("$ {command}\n(exit {code})\n");
            if !stdout.trim().is_empty() {
                out.push_str(&format!("\n--- stdout ---\n{stdout}"));
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
    vec![Box::new(ShellRun)]
}

#[cfg(test)]
mod tests {
    use super::program_base;

    #[test]
    fn basename_and_ext() {
        assert_eq!(program_base("git"), "git");
        assert_eq!(program_base("/usr/bin/git"), "git");
        assert_eq!(program_base("C:\\Program Files\\Git\\git.EXE"), "git");
        assert_eq!(program_base("cargo.cmd"), "cargo");
    }
}
