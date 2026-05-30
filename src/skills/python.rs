//! Python runner — execute Python code in a subprocess to the system
//! interpreter (`python3` / `python` / configurable). Off by default
//! (`[python].enabled`).
//!
//! **Routed through the confirmation guard on every call.** Like `shell_run`,
//! running arbitrary Python is effectively arbitrary code execution; the first
//! call returns a token, the second runs it (or `trust=true` to whitelist that
//! exact script for the session). `[python].allow_destructive` pre-authorizes
//! (skips the prompt entirely).

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::skills::guard::Decision;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, text_result};

pub const TOOL_NAMES: &[&str] = &["python_run"];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PythonRunArgs {
    /// The Python code to execute (fed to the interpreter via stdin).
    code: String,
    /// Optional command-line arguments (sys.argv[1:]).
    #[serde(default)]
    args: Option<Vec<String>>,
    /// Override the interpreter (default = `[python].interpreter`, which itself
    /// defaults to `python3` then `python`).
    #[serde(default)]
    interpreter: Option<String>,
    /// Timeout in seconds (default 30, capped at 600).
    #[serde(default)]
    timeout_secs: Option<u32>,
    /// Confirmation token returned by the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// Whitelist THIS exact code for the rest of the session (use with `confirm`).
    #[serde(default)]
    trust: Option<bool>,
}

pub struct PythonRun;
impl Skill for PythonRun {
    fn name(&self) -> &'static str {
        "python_run"
    }
    fn description(&self) -> &'static str {
        "Execute Python code in the system interpreter (`python3` by default) via stdin, with \
        a timeout. **Every call confirms first** — the first call returns a token; the second \
        runs it. `trust=true` (with the token) whitelists THIS exact code for the session; \
        `[python].allow_destructive=true` pre-authorizes any code (skip the prompt entirely). \
        Returns stdout + stderr + exit code."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PythonRunArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PythonRunArgs>()?;
            let cfg = &server.python;
            let interp = args
                .interpreter
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    Some(cfg.interpreter.clone()).filter(|s| !s.is_empty())
                })
                .unwrap_or_else(|| {
                    if cfg!(windows) {
                        "python".to_string()
                    } else {
                        "python3".to_string()
                    }
                });
            let secs = args
                .timeout_secs
                .map(|n| n as u64)
                .unwrap_or(cfg.timeout_secs)
                .clamp(1, 600);
            let preview: String = args.code.chars().take(80).collect();
            let summary = format!(
                "run Python ({}): {}{}",
                interp,
                preview,
                if args.code.chars().count() > 80 { "…" } else { "" }
            );
            // Guard on the code itself, so a different script forces a new prompt.
            let key = format!("python_run|{interp}|{}", crate::constellation::hash_key(&args.code));
            if let Decision::Challenge(msg) = server.guard.check(
                &key,
                "python_run",
                cfg.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let mut cmd = Command::new(&interp);
            cmd.arg("-").kill_on_drop(true).stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if let Some(a) = &args.args {
                for v in a {
                    cmd.arg(v);
                }
            }
            let mut child = cmd.spawn().map_err(|e| {
                internal(anyhow::anyhow!(
                    "spawn {interp}: {e} (is it installed and on PATH?)"
                ))
            })?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(args.code.as_bytes())
                    .await
                    .map_err(|e| internal(anyhow::anyhow!("write stdin: {e}")))?;
            }
            let output_fut = child.wait_with_output();
            let output = match tokio::time::timeout(Duration::from_secs(secs), output_fut).await {
                Ok(Ok(o)) => o,
                Ok(Err(e)) => return Err(internal(anyhow::anyhow!("python wait: {e}"))),
                Err(_) => {
                    return Ok(text_result(format!(
                        "Python timed out after {secs}s; process killed."
                    )))
                }
            };
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let code = output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into());
            let mut out = format!("$ {interp} (exit {code})\n");
            if !stdout.is_empty() {
                out.push_str("--- stdout ---\n");
                out.push_str(&stdout);
                if !stdout.ends_with('\n') {
                    out.push('\n');
                }
            }
            if !stderr.is_empty() {
                out.push_str("--- stderr ---\n");
                out.push_str(&stderr);
                if !stderr.ends_with('\n') {
                    out.push('\n');
                }
            }
            if stdout.is_empty() && stderr.is_empty() {
                out.push_str("(no output)\n");
            }
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(PythonRun)]
}
