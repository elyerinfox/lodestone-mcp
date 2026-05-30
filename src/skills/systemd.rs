//! Linux systemd skills — read-mostly wrapper around `systemctl` and
//! `journalctl`. Off by default (`[systemd].enabled`). Start / stop / restart
//! are routed through the confirmation guard (golden rule 8).
//!
//! Read tools (`systemd_list`, `systemd_status`, `systemd_logs`) call the
//! binaries directly. Write tools (`systemd_start`, `systemd_stop`,
//! `systemd_restart`) confirm at call time unless `[systemd].allow_destructive`
//! pre-authorizes.

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

pub const TOOL_NAMES: &[&str] = &[
    "systemd_list",
    "systemd_status",
    "systemd_logs",
    "systemd_start",
    "systemd_stop",
    "systemd_restart",
];

async fn run(cmd: &str, args: &[&str], secs: u64) -> Result<(i32, String, String), McpError> {
    let mut c = Command::new(cmd);
    c.args(args).kill_on_drop(true);
    let fut = c.output();
    let output = match tokio::time::timeout(Duration::from_secs(secs), fut).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Err(internal(anyhow::anyhow!(
                "{cmd}: {e} (is it installed on PATH? this skill only works on Linux with systemd)"
            )))
        }
        Err(_) => return Err(invalid(format!("{cmd} timed out after {secs}s"))),
    };
    Ok((
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListArgs {
    /// Filter unit type: `service`, `socket`, `timer`, `target`, … Default: service.
    #[serde(default)]
    kind: Option<String>,
    /// Only show units in this state (`failed`, `running`, etc.). Default: all.
    #[serde(default)]
    state: Option<String>,
}

pub struct SystemdList;
impl Skill for SystemdList {
    fn name(&self) -> &'static str {
        "systemd_list"
    }
    fn description(&self) -> &'static str {
        "List systemd units (`systemctl list-units`). Filter by `kind` (default `service`) and \
        optional `state` (e.g. `failed`)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ListArgs>()?;
            let kind = args.kind.unwrap_or_else(|| "service".into());
            let mut sysctl_args = vec!["list-units", "--no-pager", "--no-legend", "--type", kind.as_str()];
            if let Some(s) = &args.state {
                sysctl_args.push("--state");
                sysctl_args.push(s);
            }
            let (code, stdout, stderr) = run("systemctl", &sysctl_args, 15).await?;
            let body = if !stdout.is_empty() { stdout } else { stderr };
            Ok(text_result(truncate_chars(
                &format!("$ systemctl list-units (exit {code})\n{body}"),
                server.max_chars,
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UnitArgs {
    /// Unit name (e.g. "nginx.service").
    unit: String,
}

pub struct SystemdStatus;
impl Skill for SystemdStatus {
    fn name(&self) -> &'static str {
        "systemd_status"
    }
    fn description(&self) -> &'static str {
        "Show one systemd unit's status (`systemctl status`): active/inactive, sub-state, \
        recent log lines."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<UnitArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<UnitArgs>()?;
            let (code, stdout, stderr) =
                run("systemctl", &["status", "--no-pager", &args.unit], 15).await?;
            let body = if !stdout.is_empty() { stdout } else { stderr };
            Ok(text_result(truncate_chars(
                &format!("$ systemctl status {} (exit {code})\n{body}", args.unit),
                server.max_chars,
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LogsArgs {
    /// Unit name (e.g. "nginx.service").
    unit: String,
    /// How many lines (default 100, capped at 5000).
    #[serde(default)]
    lines: Option<u32>,
}

pub struct SystemdLogs;
impl Skill for SystemdLogs {
    fn name(&self) -> &'static str {
        "systemd_logs"
    }
    fn description(&self) -> &'static str {
        "Tail a unit's journal log (`journalctl -u <unit> -n <lines>`)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LogsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<LogsArgs>()?;
            let n = args.lines.unwrap_or(100).clamp(1, 5000).to_string();
            let (code, stdout, stderr) = run(
                "journalctl",
                &["-u", &args.unit, "-n", &n, "--no-pager"],
                30,
            )
            .await?;
            let body = if !stdout.is_empty() { stdout } else { stderr };
            Ok(text_result(truncate_chars(
                &format!("$ journalctl -u {} -n {n} (exit {code})\n{body}", args.unit),
                server.max_chars,
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ActionArgs {
    unit: String,
    #[serde(default)]
    confirm: Option<String>,
    #[serde(default)]
    trust: Option<bool>,
}

async fn act(
    server: &crate::Lodestone,
    action: &'static str,
    args: ActionArgs,
) -> Result<CallToolResult, McpError> {
    let cfg = &server.systemd;
    let key = format!("systemd:{action}:{}", args.unit);
    if let Decision::Challenge(msg) = server.guard.check(
        &key,
        match action {
            "start" => "systemd_start",
            "stop" => "systemd_stop",
            _ => "systemd_restart",
        },
        cfg.allow_destructive,
        &format!("systemctl {action} {}", args.unit),
        args.confirm.as_deref(),
        args.trust.unwrap_or(false),
    ) {
        return Ok(text_result(msg));
    }
    let (code, stdout, stderr) = run("systemctl", &[action, &args.unit], 30).await?;
    let body = if !stdout.is_empty() {
        stdout
    } else if !stderr.is_empty() {
        stderr
    } else {
        format!("systemctl {action} {} (exit {code}) — no output.", args.unit)
    };
    Ok(text_result(format!(
        "$ systemctl {action} {} (exit {code})\n{body}",
        args.unit
    )))
}

pub struct SystemdStart;
impl Skill for SystemdStart {
    fn name(&self) -> &'static str {
        "systemd_start"
    }
    fn description(&self) -> &'static str {
        "Start a systemd unit. Destructive — guarded by confirm/trust. \
        `[systemd].allow_destructive=true` pre-authorizes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ActionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ActionArgs>()?;
            act(server, "start", args).await
        })
    }
}

pub struct SystemdStop;
impl Skill for SystemdStop {
    fn name(&self) -> &'static str {
        "systemd_stop"
    }
    fn description(&self) -> &'static str {
        "Stop a systemd unit. Destructive — guarded by confirm/trust."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ActionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ActionArgs>()?;
            act(server, "stop", args).await
        })
    }
}

pub struct SystemdRestart;
impl Skill for SystemdRestart {
    fn name(&self) -> &'static str {
        "systemd_restart"
    }
    fn description(&self) -> &'static str {
        "Restart a systemd unit. Destructive — guarded by confirm/trust."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ActionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ActionArgs>()?;
            act(server, "restart", args).await
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SystemdList),
        Box::new(SystemdStatus),
        Box::new(SystemdLogs),
        Box::new(SystemdStart),
        Box::new(SystemdStop),
        Box::new(SystemdRestart),
    ]
}
