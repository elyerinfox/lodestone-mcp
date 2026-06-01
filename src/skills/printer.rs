//! Printer skill — list printers and print text via the OS print system. **Off by
//! default** (`[printer].enabled`). There is no good cross-platform Rust printing
//! crate, so this shells out: CUPS `lp`/`lpstat` on Unix, PowerShell
//! `Get-Printer`/`Out-Printer` on Windows. Printing is side-effecting, so
//! `printer_print` goes through the confirmation [`guard`](crate::skills::guard).

use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::skills::guard::Decision;
use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::{internal, text_result};

/// Tool names (gated by `[printer].enabled` in `disabled_by_config`).
pub const TOOL_NAMES: &[&str] = &["printer_list", "printer_print"];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PrintArgs {
    /// Text to print.
    text: String,
    /// Printer name (as shown by printer_list). Omit for the system default.
    #[serde(default)]
    printer: Option<String>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for printer_print for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct PrinterList;
impl Skill for PrinterList {
    fn name(&self) -> &'static str {
        "printer_list"
    }
    fn description(&self) -> &'static str {
        "List the printers known to this machine's print system (CUPS on Unix, the Windows \
        spooler). Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let out = list_printers().await.map_err(internal)?;
            Ok(text_result(out))
        })
    }
}

pub struct PrinterPrint;
impl Skill for PrinterPrint {
    fn name(&self) -> &'static str {
        "printer_print"
    }
    fn description(&self) -> &'static str {
        "Print text to a printer (off by default; [printer]). Side-effecting — the first call \
        returns a confirmation token and prints nothing; call again with confirm=<token> to print \
        (or confirm + trust=true). Omit `printer` for the system default."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PrintArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PrintArgs>()?;
            let target = args.printer.as_deref().unwrap_or("default printer");
            let summary = format!("print {} char(s) to {target}", args.text.len());
            if let Decision::Challenge(msg) = server.guard.check(
                "printer_print",
                "printer_print",
                false,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            print_text(&args.text, args.printer.as_deref())
                .await
                .map_err(internal)?;
            Ok(text_result(format!(
                "Sent {} char(s) to {target}.",
                args.text.len()
            )))
        })
    }
}

#[cfg(windows)]
async fn list_printers() -> Result<String> {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-Printer | Select-Object -ExpandProperty Name",
        ])
        .output()
        .await
        .map_err(|e| anyhow!("could not run PowerShell to list printers: {e}"))?;
    let body = String::from_utf8_lossy(&out.stdout);
    let names: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return Ok("No printers found.".into());
    }
    Ok(format!(
        "Printers ({}):\n  {}",
        names.len(),
        names.join("\n  ")
    ))
}

#[cfg(not(windows))]
async fn list_printers() -> Result<String> {
    let out = Command::new("lpstat")
        .arg("-e")
        .output()
        .await
        .map_err(|e| anyhow!("could not run `lpstat` (is CUPS installed?): {e}"))?;
    let body = String::from_utf8_lossy(&out.stdout);
    let names: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if names.is_empty() {
        return Ok("No printers found (lpstat -e returned nothing).".into());
    }
    Ok(format!(
        "Printers ({}):\n  {}",
        names.len(),
        names.join("\n  ")
    ))
}

#[cfg(windows)]
async fn print_text(text: &str, printer: Option<&str>) -> Result<()> {
    // Pipe the text to Out-Printer (optionally to a named printer).
    let cmd = match printer {
        Some(p) => format!("$input | Out-Printer -Name \"{}\"", p.replace('"', "")),
        None => "$input | Out-Printer".to_string(),
    };
    run_with_stdin(
        "powershell",
        &["-NoProfile", "-Command", &cmd],
        text.as_bytes(),
    )
    .await
}

#[cfg(not(windows))]
async fn print_text(text: &str, printer: Option<&str>) -> Result<()> {
    let mut args: Vec<String> = Vec::new();
    if let Some(p) = printer {
        args.push("-d".into());
        args.push(p.to_string());
    }
    let argrefs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_with_stdin("lp", &argrefs, text.as_bytes()).await
}

/// Run a command, feeding `stdin`, and map a non-zero exit to an error.
async fn run_with_stdin(program: &str, args: &[&str], stdin: &[u8]) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!("'{program}' not found — no print system available")
            } else {
                anyhow!("could not start '{program}': {e}")
            }
        })?;
    if let Some(mut si) = child.stdin.take() {
        si.write_all(stdin).await.ok();
        si.shutdown().await.ok();
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| anyhow!("print command failed: {e}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "print command exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "printer"
    }
    fn tools(&self) -> &'static [&'static str] {
        TOOL_NAMES
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::SkillCapability;
        // The OS print stack: CUPS via `lpstat`/`lp` on Unix, Windows
        // via builtin print API. Containers without CUPS report
        // unavailable so operators know they need to install
        // cups-client or extend the image.
        #[cfg(windows)]
        {
            SkillCapability::Ready
        }
        #[cfg(not(windows))]
        {
            if crate::skills::binary_on_path("lpstat") {
                SkillCapability::Ready
            } else {
                SkillCapability::unavailable(
                    "no `lpstat` on PATH (CUPS missing)",
                    "install cups-client (apt/brew/dnf) or extend the container image",
                )
            }
        }
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(PrinterList), Box::new(PrinterPrint)]
}
