//! Package manager skill — one set of tools that target every supported
//! package manager via an explicit `kind` argument. **Off by default**
//! (`[packages].enabled`). Destructive operations (`package_install`,
//! `package_upgrade`, `package_remove`) route through the confirmation
//! [`guard`](crate::skills::guard) (golden rule 8); `[packages].
//! allow_destructive` pre-authorizes.
//!
//! Why one tool per method with a `kind` arg, not one tool per (method,
//! PM) pair? Different package managers are different **targets**, not
//! different methodologies — the model says "install vscode via winget"
//! the same way it says "query Postgres via this connection URL" (golden
//! rule 9, the explicit-target exception). Adding a new PM is one
//! [`Pm`] enum variant + its command lookup; no new tool surface.
//!
//! **Never `sudo`.** Privilege escalation is the operator's choice
//! (`make install` user, sudo wrapper, container user) — lodestone runs
//! whatever it was started with. Calls that fail for lack of privilege
//! return the underlying error verbatim with a hint.
//!
//! Capability gating: the family is `Ready` if **any** supported PM is
//! on `$PATH`. Per-tool: the wrapper looks at the `kind` argument and
//! refuses cleanly when that specific PM isn't installed.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use tokio::process::Command;

use crate::skills::guard::Decision;
use crate::skills::{binary_on_path, schema_for, NoArgs, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result};

/// Bound the time we'll wait for any single PM invocation. Most search /
/// info / list operations finish in a few seconds; installs / upgrades
/// can be slow on big package sets. We cap at 10 min so a stuck mirror
/// can't lock up a process.
const CMD_TIMEOUT: Duration = Duration::from_secs(600);

/// One supported package manager. The variants below are matched by the
/// `kind` arg verbatim (case-insensitive). Adding one means: another
/// variant, a match arm in each `cmd_*` builder, and an entry in
/// [`Pm::ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pm {
    Winget,
    Chocolatey,
    Apt,
    Dnf,
    Yum,
    Apk,
    Pacman,
    /// AUR wrapper around pacman. Distinct kind so the model can opt
    /// into AUR explicitly (community-maintained PKGBUILDs); falls back
    /// to plain pacman if `yay` isn't installed.
    Yay,
    Brew,
    Zypper,
    /// FreeBSD's `pkg`. Distinct from Alpine's `apk` despite the short name.
    Pkg,
}

impl Pm {
    /// Every supported PM, in display order. Iteration order is
    /// stable for `package_managers` output.
    pub const ALL: &'static [Pm] = &[
        Pm::Winget,
        Pm::Chocolatey,
        Pm::Brew,
        Pm::Apt,
        Pm::Dnf,
        Pm::Yum,
        Pm::Apk,
        Pm::Pacman,
        Pm::Yay,
        Pm::Zypper,
        Pm::Pkg,
    ];

    /// Stable id used in the `kind` argument and in output rows.
    pub fn id(self) -> &'static str {
        match self {
            Pm::Winget => "winget",
            Pm::Chocolatey => "chocolatey",
            Pm::Apt => "apt",
            Pm::Dnf => "dnf",
            Pm::Yum => "yum",
            Pm::Apk => "apk",
            Pm::Pacman => "pacman",
            Pm::Yay => "yay",
            Pm::Brew => "brew",
            Pm::Zypper => "zypper",
            Pm::Pkg => "pkg",
        }
    }

    /// Executable name to probe on `$PATH`. `Chocolatey` ships as `choco`.
    pub fn binary(self) -> &'static str {
        match self {
            Pm::Winget => "winget",
            Pm::Chocolatey => "choco",
            Pm::Apt => "apt",
            Pm::Dnf => "dnf",
            Pm::Yum => "yum",
            Pm::Apk => "apk",
            Pm::Pacman => "pacman",
            Pm::Yay => "yay",
            Pm::Brew => "brew",
            Pm::Zypper => "zypper",
            Pm::Pkg => "pkg",
        }
    }

    /// One-line description of the manager + its scope.
    pub fn label(self) -> &'static str {
        match self {
            Pm::Winget => "Windows Package Manager",
            Pm::Chocolatey => "Chocolatey (Windows)",
            Pm::Apt => "Debian/Ubuntu APT",
            Pm::Dnf => "Fedora/RHEL DNF",
            Pm::Yum => "RHEL/CentOS YUM",
            Pm::Apk => "Alpine APK",
            Pm::Pacman => "Arch Pacman (official repos)",
            Pm::Yay => "AUR (yay, community PKGBUILDs)",
            Pm::Brew => "Homebrew (macOS / Linux)",
            Pm::Zypper => "openSUSE Zypper",
            Pm::Pkg => "FreeBSD pkg",
        }
    }

    /// Case-insensitive parse from the `kind` argument.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "winget" => Some(Pm::Winget),
            "chocolatey" | "choco" => Some(Pm::Chocolatey),
            "apt" | "apt-get" => Some(Pm::Apt),
            "dnf" => Some(Pm::Dnf),
            "yum" => Some(Pm::Yum),
            "apk" => Some(Pm::Apk),
            "pacman" => Some(Pm::Pacman),
            "yay" | "aur" => Some(Pm::Yay),
            "brew" | "homebrew" => Some(Pm::Brew),
            "zypper" => Some(Pm::Zypper),
            "pkg" => Some(Pm::Pkg),
            _ => None,
        }
    }

    fn cmd_search(self, query: &str) -> Vec<String> {
        let q = query.to_string();
        match self {
            Pm::Winget => vec!["winget".into(), "search".into(), q],
            Pm::Chocolatey => vec!["choco".into(), "search".into(), q],
            Pm::Apt => vec!["apt-cache".into(), "search".into(), q],
            Pm::Dnf => vec!["dnf".into(), "search".into(), q],
            Pm::Yum => vec!["yum".into(), "search".into(), q],
            Pm::Apk => vec!["apk".into(), "search".into(), q],
            Pm::Pacman => vec!["pacman".into(), "-Ss".into(), q],
            Pm::Yay => vec!["yay".into(), "-Ss".into(), q],
            Pm::Brew => vec!["brew".into(), "search".into(), q],
            Pm::Zypper => vec!["zypper".into(), "search".into(), q],
            Pm::Pkg => vec!["pkg".into(), "search".into(), q],
        }
    }

    fn cmd_info(self, name: &str) -> Vec<String> {
        let n = name.to_string();
        match self {
            Pm::Winget => vec!["winget".into(), "show".into(), n],
            Pm::Chocolatey => vec!["choco".into(), "info".into(), n],
            Pm::Apt => vec!["apt-cache".into(), "show".into(), n],
            Pm::Dnf => vec!["dnf".into(), "info".into(), n],
            Pm::Yum => vec!["yum".into(), "info".into(), n],
            Pm::Apk => vec!["apk".into(), "info".into(), "-a".into(), n],
            Pm::Pacman => vec!["pacman".into(), "-Si".into(), n],
            Pm::Yay => vec!["yay".into(), "-Si".into(), n],
            Pm::Brew => vec!["brew".into(), "info".into(), n],
            Pm::Zypper => vec!["zypper".into(), "info".into(), n],
            Pm::Pkg => vec!["pkg".into(), "info".into(), n],
        }
    }

    fn cmd_list_installed(self) -> Vec<String> {
        match self {
            Pm::Winget => vec!["winget".into(), "list".into()],
            Pm::Chocolatey => vec!["choco".into(), "list".into(), "--local-only".into()],
            Pm::Apt => vec!["apt".into(), "list".into(), "--installed".into()],
            Pm::Dnf => vec!["dnf".into(), "list".into(), "installed".into()],
            Pm::Yum => vec!["yum".into(), "list".into(), "installed".into()],
            Pm::Apk => vec!["apk".into(), "info".into(), "-v".into()],
            Pm::Pacman | Pm::Yay => vec!["pacman".into(), "-Q".into()],
            Pm::Brew => vec!["brew".into(), "list".into(), "--versions".into()],
            Pm::Zypper => vec!["zypper".into(), "search".into(), "--installed-only".into()],
            Pm::Pkg => vec!["pkg".into(), "info".into()],
        }
    }

    fn cmd_updates(self) -> Vec<String> {
        match self {
            Pm::Winget => vec!["winget".into(), "upgrade".into()],
            Pm::Chocolatey => vec!["choco".into(), "outdated".into()],
            Pm::Apt => vec!["apt".into(), "list".into(), "--upgradable".into()],
            Pm::Dnf => vec!["dnf".into(), "check-update".into()],
            Pm::Yum => vec!["yum".into(), "check-update".into()],
            Pm::Apk => vec!["apk".into(), "version".into(), "-l".into(), "<".into()],
            Pm::Pacman | Pm::Yay => vec!["pacman".into(), "-Qu".into()],
            Pm::Brew => vec!["brew".into(), "outdated".into()],
            Pm::Zypper => vec!["zypper".into(), "list-updates".into()],
            Pm::Pkg => vec!["pkg".into(), "upgrade".into(), "-n".into()],
        }
    }

    /// Non-interactive install command. Caller must already have routed
    /// through the guard.
    fn cmd_install(self, name: &str) -> Vec<String> {
        let n = name.to_string();
        match self {
            Pm::Winget => vec![
                "winget".into(),
                "install".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
                n,
            ],
            Pm::Chocolatey => vec![
                "choco".into(),
                "install".into(),
                n,
                "-y".into(),
                "--no-progress".into(),
            ],
            Pm::Apt => vec!["apt-get".into(), "install".into(), "-y".into(), n],
            Pm::Dnf => vec!["dnf".into(), "install".into(), "-y".into(), n],
            Pm::Yum => vec!["yum".into(), "install".into(), "-y".into(), n],
            Pm::Apk => vec!["apk".into(), "add".into(), n],
            Pm::Pacman => vec!["pacman".into(), "-S".into(), "--noconfirm".into(), n],
            Pm::Yay => vec!["yay".into(), "-S".into(), "--noconfirm".into(), n],
            Pm::Brew => vec!["brew".into(), "install".into(), n],
            Pm::Zypper => vec![
                "zypper".into(),
                "--non-interactive".into(),
                "install".into(),
                n,
            ],
            Pm::Pkg => vec!["pkg".into(), "install".into(), "-y".into(), n],
        }
    }

    /// Non-interactive upgrade. `name = None` → upgrade-all where the PM
    /// supports it (the rest run their default upgrade which is usually
    /// equivalent).
    fn cmd_upgrade(self, name: Option<&str>) -> Vec<String> {
        let n = name.map(|s| s.to_string());
        match (self, n) {
            (Pm::Winget, Some(name)) => vec![
                "winget".into(),
                "upgrade".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
                name,
            ],
            (Pm::Winget, None) => vec![
                "winget".into(),
                "upgrade".into(),
                "--all".into(),
                "--silent".into(),
                "--accept-source-agreements".into(),
                "--accept-package-agreements".into(),
            ],
            (Pm::Chocolatey, Some(name)) => vec![
                "choco".into(),
                "upgrade".into(),
                name,
                "-y".into(),
                "--no-progress".into(),
            ],
            (Pm::Chocolatey, None) => vec![
                "choco".into(),
                "upgrade".into(),
                "all".into(),
                "-y".into(),
                "--no-progress".into(),
            ],
            (Pm::Apt, Some(name)) => vec![
                "apt-get".into(),
                "install".into(),
                "--only-upgrade".into(),
                "-y".into(),
                name,
            ],
            (Pm::Apt, None) => vec!["apt-get".into(), "upgrade".into(), "-y".into()],
            (Pm::Dnf, Some(name)) => vec!["dnf".into(), "upgrade".into(), "-y".into(), name],
            (Pm::Dnf, None) => vec!["dnf".into(), "upgrade".into(), "-y".into()],
            (Pm::Yum, Some(name)) => vec!["yum".into(), "update".into(), "-y".into(), name],
            (Pm::Yum, None) => vec!["yum".into(), "update".into(), "-y".into()],
            (Pm::Apk, Some(name)) => vec!["apk".into(), "upgrade".into(), name],
            (Pm::Apk, None) => vec!["apk".into(), "upgrade".into()],
            (Pm::Pacman, Some(name)) => {
                vec!["pacman".into(), "-S".into(), "--noconfirm".into(), name]
            }
            (Pm::Pacman, None) => vec!["pacman".into(), "-Syu".into(), "--noconfirm".into()],
            (Pm::Yay, Some(name)) => vec!["yay".into(), "-S".into(), "--noconfirm".into(), name],
            (Pm::Yay, None) => vec!["yay".into(), "-Syu".into(), "--noconfirm".into()],
            (Pm::Brew, Some(name)) => vec!["brew".into(), "upgrade".into(), name],
            (Pm::Brew, None) => vec!["brew".into(), "upgrade".into()],
            (Pm::Zypper, Some(name)) => vec![
                "zypper".into(),
                "--non-interactive".into(),
                "update".into(),
                name,
            ],
            (Pm::Zypper, None) => {
                vec!["zypper".into(), "--non-interactive".into(), "update".into()]
            }
            (Pm::Pkg, Some(name)) => vec!["pkg".into(), "upgrade".into(), "-y".into(), name],
            (Pm::Pkg, None) => vec!["pkg".into(), "upgrade".into(), "-y".into()],
        }
    }

    fn cmd_remove(self, name: &str) -> Vec<String> {
        let n = name.to_string();
        match self {
            Pm::Winget => vec!["winget".into(), "uninstall".into(), "--silent".into(), n],
            Pm::Chocolatey => vec![
                "choco".into(),
                "uninstall".into(),
                n,
                "-y".into(),
                "--no-progress".into(),
            ],
            Pm::Apt => vec!["apt-get".into(), "remove".into(), "-y".into(), n],
            Pm::Dnf => vec!["dnf".into(), "remove".into(), "-y".into(), n],
            Pm::Yum => vec!["yum".into(), "remove".into(), "-y".into(), n],
            Pm::Apk => vec!["apk".into(), "del".into(), n],
            Pm::Pacman => vec!["pacman".into(), "-R".into(), "--noconfirm".into(), n],
            Pm::Yay => vec!["yay".into(), "-R".into(), "--noconfirm".into(), n],
            Pm::Brew => vec!["brew".into(), "uninstall".into(), n],
            Pm::Zypper => vec![
                "zypper".into(),
                "--non-interactive".into(),
                "remove".into(),
                n,
            ],
            Pm::Pkg => vec!["pkg".into(), "delete".into(), "-y".into(), n],
        }
    }
}

/// Resolve the `kind` argument into a `Pm` or return an LLM-facing
/// error listing every supported value.
fn parse_kind(kind: &str) -> Result<Pm, McpError> {
    Pm::parse(kind).ok_or_else(|| {
        let all: Vec<&str> = Pm::ALL.iter().map(|p| p.id()).collect();
        invalid(format!(
            "unknown package manager '{kind}'. Supported: {}",
            all.join(", ")
        ))
    })
}

/// Confirm the PM's binary is on `$PATH` before we spawn — gives the
/// LLM a clear "winget not installed" message rather than a vague
/// process-spawn error.
fn require_binary(pm: Pm) -> Result<(), McpError> {
    if binary_on_path(pm.binary()) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{} ({}) isn't installed or not on $PATH on this host",
            pm.label(),
            pm.binary()
        )))
    }
}

/// Run a PM command with a bounded timeout. Captures stdout + stderr;
/// the PM family routinely emits useful detail to stderr (apt's
/// "WARNING: apt does not have a stable CLI" goes there, for example).
async fn run_cmd(argv: Vec<String>) -> Result<String> {
    if argv.is_empty() {
        return Err(anyhow!("empty command"));
    }
    let program = argv[0].clone();
    let args: Vec<String> = argv[1..].to_vec();
    let child = Command::new(&program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!(
                    "'{program}' not found on PATH — the package manager isn't installed \
                     in this environment"
                )
            } else {
                anyhow!("could not start '{program}': {e}")
            }
        })?;
    let out = match tokio::time::timeout(CMD_TIMEOUT, child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| anyhow!("'{program}' failed: {e}"))?,
        Err(_) => {
            return Err(anyhow!(
                "'{program}' timed out after {}s",
                CMD_TIMEOUT.as_secs()
            ))
        }
    };
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(err.trim());
    }
    if !out.status.success() {
        return Err(anyhow!(
            "'{program}' exited with status {}: {}",
            out.status.code().unwrap_or(-1),
            text.trim()
        ));
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// Read-only tools
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct KindOnlyArgs {
    /// Package manager kind. One of `winget`, `chocolatey`, `apt`, `dnf`,
    /// `yum`, `apk`, `pacman`, `yay`, `brew`, `zypper`, `pkg`.
    kind: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    /// Package manager kind.
    kind: String,
    /// Search query (forwarded to the PM verbatim — most accept substring).
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NameArgs {
    /// Package manager kind.
    kind: String,
    /// Package name as the PM expects it.
    name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DestructiveArgs {
    /// Package manager kind.
    kind: String,
    /// Package name (omit for `package_upgrade` to upgrade everything).
    #[serde(default)]
    name: Option<String>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for this exact PM operation for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct PackageManagers;
impl Skill for PackageManagers {
    fn name(&self) -> &'static str {
        "package_managers"
    }
    fn description(&self) -> &'static str {
        "List package managers detected on the host (binary on $PATH). Returns each \
        with its `kind` id (the value you pass to the other `package_*` tools), its \
        binary name, and a one-line label. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let mut rows: Vec<String> = Vec::new();
            for pm in Pm::ALL {
                let present = binary_on_path(pm.binary());
                rows.push(format!(
                    "  {:<11} {} — {} ({})",
                    pm.id(),
                    if present { "✓" } else { "·" },
                    pm.label(),
                    pm.binary()
                ));
            }
            let count = Pm::ALL
                .iter()
                .filter(|p| binary_on_path(p.binary()))
                .count();
            Ok(text_result(format!(
                "{count} package manager(s) detected on this host (✓ = present):\n{}",
                rows.join("\n")
            )))
        })
    }
}

pub struct PackageSearch;
impl Skill for PackageSearch {
    fn name(&self) -> &'static str {
        "package_search"
    }
    fn description(&self) -> &'static str {
        "Search a package manager for packages matching `query`. `kind` is one of the \
        ids `package_managers` lists. Output is the PM's raw search format (one row \
        per hit on most). Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SearchArgs>()?;
            let pm = parse_kind(&args.kind)?;
            require_binary(pm)?;
            let q = args.query.trim();
            if q.is_empty() {
                return Err(invalid("query is required"));
            }
            let cmd = pm.cmd_search(q);
            let out = run_cmd(cmd).await.map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub struct PackageInfo;
impl Skill for PackageInfo {
    fn name(&self) -> &'static str {
        "package_info"
    }
    fn description(&self) -> &'static str {
        "Show package metadata (version, description, dependencies, source URL where \
        the PM exposes one) for one named package. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NameArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<NameArgs>()?;
            let pm = parse_kind(&args.kind)?;
            require_binary(pm)?;
            if args.name.trim().is_empty() {
                return Err(invalid("name is required"));
            }
            let cmd = pm.cmd_info(args.name.trim());
            let out = run_cmd(cmd).await.map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub struct PackageList;
impl Skill for PackageList {
    fn name(&self) -> &'static str {
        "package_list"
    }
    fn description(&self) -> &'static str {
        "List packages currently installed via the named manager. Output may be long; \
        truncated to `[retrieval].max_chars`. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<KindOnlyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<KindOnlyArgs>()?;
            let pm = parse_kind(&args.kind)?;
            require_binary(pm)?;
            let cmd = pm.cmd_list_installed();
            let out = run_cmd(cmd).await.map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub struct PackageUpdates;
impl Skill for PackageUpdates {
    fn name(&self) -> &'static str {
        "package_updates"
    }
    fn description(&self) -> &'static str {
        "Show packages with available updates without applying anything. PM-native \
        check (e.g. `apt list --upgradable`, `brew outdated`, `dnf check-update`). \
        Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<KindOnlyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<KindOnlyArgs>()?;
            let pm = parse_kind(&args.kind)?;
            require_binary(pm)?;
            let cmd = pm.cmd_updates();
            // Some PMs use non-zero exit to signal "updates available"
            // (dnf check-update is the canonical one) — surface stdout
            // even when the command "fails" so the model sees the list.
            match run_cmd(cmd).await {
                Ok(out) => Ok(text_result(truncate_chars(&out, server.max_chars))),
                Err(e) => {
                    let msg = format!("{e:#}");
                    Ok(text_result(truncate_chars(&msg, server.max_chars)))
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Destructive tools — guard-gated.
// ---------------------------------------------------------------------------

/// Guard helper: run the confirmation gate, return Some(prompt) when
/// we need to challenge, None when authorized to proceed.
fn guard_check(
    server: &crate::Lodestone,
    pre_authorize: bool,
    action_tag: &str,
    summary: &str,
    confirm: Option<&str>,
    trust: bool,
) -> Option<String> {
    if let Decision::Challenge(msg) = server.guard.check(
        action_tag,
        action_tag,
        pre_authorize,
        summary,
        confirm,
        trust,
    ) {
        Some(msg)
    } else {
        None
    }
}

pub struct PackageInstall;
impl Skill for PackageInstall {
    fn name(&self) -> &'static str {
        "package_install"
    }
    fn description(&self) -> &'static str {
        "Install a package via the named manager. Side-effecting — the first call \
        returns a confirmation token and does nothing; call again with `confirm=<token>` \
        to install (or `confirm + trust=true` to skip future prompts this session). \
        `[packages].allow_destructive=true` pre-authorizes. Lodestone never `sudo`s; \
        privilege escalation is the operator's choice."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DestructiveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DestructiveArgs>()?;
            let pm = parse_kind(&args.kind)?;
            require_binary(pm)?;
            let name = args
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("name is required for package_install"))?;
            let summary = format!("install '{name}' via {}", pm.id());
            if let Some(msg) = guard_check(
                server,
                server.cfg.packages.allow_destructive,
                "package_install",
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let cmd = pm.cmd_install(name);
            let out = run_cmd(cmd).await.map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub struct PackageUpgrade;
impl Skill for PackageUpgrade {
    fn name(&self) -> &'static str {
        "package_upgrade"
    }
    fn description(&self) -> &'static str {
        "Upgrade one named package (or every installed package when `name` is omitted). \
        Side-effecting — `confirm`/`trust` flow as for `package_install`. \
        `[packages].allow_destructive=true` pre-authorizes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DestructiveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DestructiveArgs>()?;
            let pm = parse_kind(&args.kind)?;
            require_binary(pm)?;
            let name = args
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let summary = match name {
                Some(n) => format!("upgrade '{n}' via {}", pm.id()),
                None => format!("upgrade ALL packages via {}", pm.id()),
            };
            if let Some(msg) = guard_check(
                server,
                server.cfg.packages.allow_destructive,
                "package_upgrade",
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let cmd = pm.cmd_upgrade(name);
            let out = run_cmd(cmd).await.map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub struct PackageRemove;
impl Skill for PackageRemove {
    fn name(&self) -> &'static str {
        "package_remove"
    }
    fn description(&self) -> &'static str {
        "Remove an installed package. Side-effecting — `confirm`/`trust` flow as for \
        `package_install`. `[packages].allow_destructive=true` pre-authorizes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DestructiveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DestructiveArgs>()?;
            let pm = parse_kind(&args.kind)?;
            require_binary(pm)?;
            let name = args
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("name is required for package_remove"))?;
            let summary = format!("remove '{name}' via {}", pm.id());
            if let Some(msg) = guard_check(
                server,
                server.cfg.packages.allow_destructive,
                "package_remove",
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let cmd = pm.cmd_remove(name);
            let out = run_cmd(cmd).await.map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(PackageManagers),
        Box::new(PackageSearch),
        Box::new(PackageInfo),
        Box::new(PackageList),
        Box::new(PackageUpdates),
        Box::new(PackageInstall),
        Box::new(PackageUpgrade),
        Box::new(PackageRemove),
    ]
}

/// Family probe: Ready iff at least one supported PM binary is on PATH.
/// Per-tool we then check the specific `kind` at call time. The family
/// being Unavailable means "this host has none of the managers Lodestone
/// knows about" — fair to surface as a dashboard badge.
pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "packages"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Distro / OS package managers — search / info / list / updates and (with \
         confirmation) install / upgrade / remove against winget, chocolatey, apt, \
         dnf, yum, apk, pacman, yay (AUR), brew, zypper, pkg. Never `sudo`s; \
         privilege escalation is the operator's choice."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::SkillCapability;
        if Pm::ALL.iter().any(|p| binary_on_path(p.binary())) {
            SkillCapability::Ready
        } else {
            SkillCapability::unavailable(
                "no supported package manager binary found on $PATH",
                "install one of: winget, choco, apt, dnf, yum, apk, pacman, yay, brew, zypper, pkg",
            )
        }
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `package_managers {}` to see which managers exist on this host.\n\
             2. `package_search { kind: \"apt\", query: \"ripgrep\" }` to find the right package name.\n\
             3. `package_info { kind: \"apt\", name: \"ripgrep\" }` to confirm version + description.\n\
             4. `package_install { kind: \"apt\", name: \"ripgrep\" }` (confirm on second call) to install it.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_parses_canonical_and_aliases() {
        assert_eq!(Pm::parse("winget"), Some(Pm::Winget));
        assert_eq!(Pm::parse("CHOCO"), Some(Pm::Chocolatey));
        assert_eq!(Pm::parse("chocolatey"), Some(Pm::Chocolatey));
        assert_eq!(Pm::parse("apt-get"), Some(Pm::Apt));
        assert_eq!(Pm::parse("aur"), Some(Pm::Yay));
        assert_eq!(Pm::parse("homebrew"), Some(Pm::Brew));
        assert_eq!(Pm::parse("nope"), None);
    }

    #[test]
    fn every_pm_has_distinct_id_and_binary() {
        use std::collections::HashSet;
        let mut ids: HashSet<&str> = HashSet::new();
        for pm in Pm::ALL {
            assert!(ids.insert(pm.id()), "duplicate id {}", pm.id());
        }
        // Yay and Pacman both ship `pacman`-backed install paths but
        // their *primary* binary differs (yay vs pacman) — that's why
        // the kinds are split.
        assert_ne!(Pm::Yay.binary(), Pm::Pacman.binary());
    }

    #[test]
    fn search_command_includes_query_for_each_pm() {
        // Each PM's search command must (a) be non-empty, (b) carry the
        // query as one of its argv entries. The leading program name can
        // vary from `pm.binary()` — apt uses `apt-cache search`, choco
        // uses `choco` (we probe with "choco" too), etc.
        for pm in Pm::ALL {
            let cmd = pm.cmd_search("htop");
            assert!(!cmd.is_empty(), "{} has empty search cmd", pm.id());
            assert!(
                cmd.iter().any(|a| a.contains("htop")),
                "{} search cmd missing query: {cmd:?}",
                pm.id()
            );
        }
    }

    #[test]
    fn install_command_includes_non_interactive_flag() {
        // Sanity: every PM's install command pins down a non-interactive
        // flag (or uses a PM that's non-interactive by default). Stops
        // the dispatch from hanging on a y/N prompt.
        let non_interactive_markers: &[&str] =
            &["--silent", "-y", "--noconfirm", "--non-interactive"];
        for pm in Pm::ALL {
            let cmd = pm.cmd_install("pkg");
            // brew and apk are non-interactive by default; everything
            // else must carry an explicit flag.
            if matches!(pm, Pm::Brew | Pm::Apk) {
                continue;
            }
            assert!(
                cmd.iter()
                    .any(|a| non_interactive_markers.iter().any(|m| a == m)),
                "{} install lacks a non-interactive flag: {cmd:?}",
                pm.id()
            );
        }
    }
}
