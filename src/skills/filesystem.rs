//! Local filesystem skills — read and edit files/directories on the machine.
//!
//! DANGEROUS and **off by default** (golden rule: everything is gateable; this one
//! must be explicitly granted). Gated by `[filesystem].enabled`. Destructive ops
//! (`fs_write` overwriting an existing file, `fs_edit`, `fs_delete`, `fs_move`)
//! always route through the confirmation guard; `[filesystem].allow_destructive`
//! skips the prompt.
//!
//! Every path is **confined** to the configured `[filesystem].roots` (default: the
//! server's working directory). `..` components are rejected and symlinks are
//! resolved (`canonicalize`), so operations cannot escape a root. I/O runs on the
//! async runtime via `tokio::fs` (directory walks go through `spawn_blocking`).

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::config::Filesystem;
use crate::skills::guard::Decision;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result};

/// Canonicalized allowed roots (configured, or the CWD when none set).
fn roots(fs: &Filesystem) -> Result<Vec<PathBuf>, String> {
    let raw: Vec<PathBuf> = if fs.roots.is_empty() {
        vec![std::env::current_dir().map_err(|e| format!("cannot determine working dir: {e}"))?]
    } else {
        fs.roots.iter().map(PathBuf::from).collect()
    };
    let mut out = Vec::new();
    for r in raw {
        let c = r
            .canonicalize()
            .map_err(|e| format!("filesystem root '{}' is not accessible: {e}", r.display()))?;
        out.push(c);
    }
    Ok(out)
}

/// Resolve `path` (relative to the primary root, or absolute) to a real path that
/// is provably inside one of `roots`. Rejects `..` and symlink escapes.
fn confine(roots: &[PathBuf], path: &str) -> Result<PathBuf, String> {
    let p = Path::new(path.trim());
    if p.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("path must not contain '..'".into());
    }
    let primary = roots.first().ok_or("no filesystem roots configured")?;
    let candidate = if p.is_absolute() {
        p.to_path_buf()
    } else {
        primary.join(p)
    };

    // Canonicalize the longest existing prefix (resolves symlinks); the remaining
    // tail can't contain `..` (rejected above), so it stays inside.
    let mut existing: &Path = &candidate;
    let mut tail: Vec<&OsStr> = Vec::new();
    let canon_prefix = loop {
        match existing.canonicalize() {
            Ok(c) => break c,
            Err(_) => {
                let name = existing.file_name().ok_or("invalid path")?;
                tail.push(name);
                existing = existing
                    .parent()
                    .ok_or("no existing parent within a root")?;
                if existing.as_os_str().is_empty() {
                    return Err("invalid path".into());
                }
            }
        }
    };
    if !roots.iter().any(|r| canon_prefix.starts_with(r)) {
        return Err("path is outside the allowed filesystem root(s)".into());
    }
    let mut full = canon_prefix;
    for name in tail.iter().rev() {
        full.push(name);
    }
    Ok(full)
}

/// `confine`, mapped to an MCP error. Reused by other skills (e.g. ffmpeg) that
/// must keep file paths inside the configured `[filesystem].roots`.
pub(crate) fn resolve(fs: &Filesystem, path: &str) -> Result<PathBuf, McpError> {
    let roots = roots(fs).map_err(invalid)?;
    confine(&roots, path).map_err(invalid)
}

/// Wildcard (`*`) match, case-insensitive; without `*` it's a substring test.
fn matches(pat: &str, hay: &str) -> bool {
    let hay = hay.to_ascii_lowercase();
    let pat = pat.to_ascii_lowercase();
    if !pat.contains('*') {
        return hay.contains(&pat);
    }
    let (p, t): (Vec<char>, Vec<char>) = (pat.chars().collect(), hay.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, None, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

// --- argument schemas -------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadArgs {
    /// File path (relative to a configured root, or an absolute path inside one).
    path: String,
    /// Max characters of text to return. Omit for the server default.
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathArg {
    /// A path relative to a configured root, or an absolute path inside one.
    /// Defaults to the root itself when omitted.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StatArgs {
    /// A path relative to a configured root, or an absolute path inside one.
    path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FindArgs {
    /// Name/path pattern. `*` is a wildcard (e.g. `*.rs`, `src/*config*`); without
    /// `*` it matches any path containing the text.
    pattern: String,
    /// Directory to search under (relative to a root, or absolute inside one).
    /// Defaults to the primary root.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WriteArgs {
    /// Destination file path (created or overwritten). Parent dirs must exist.
    path: String,
    /// The full file contents to write.
    content: String,
    /// One-time token from a prior call's confirmation prompt. Only checked
    /// when the target file already exists (overwrite is destructive).
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EditArgs {
    /// File to edit.
    path: String,
    /// Exact text to replace. Must occur exactly once unless `replace_all`.
    old_string: String,
    /// Replacement text.
    new_string: String,
    /// Replace every occurrence instead of requiring a unique match.
    #[serde(default)]
    replace_all: Option<bool>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DeleteArgs {
    /// Path to delete.
    path: String,
    /// Recursively delete a non-empty directory (default false).
    #[serde(default)]
    recursive: Option<bool>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MoveArgs {
    /// Source path (must exist, inside a root).
    source: String,
    /// Destination path (inside a root). Overwrites if it exists.
    dest: String,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

// --- skills -----------------------------------------------------------------

pub struct FsRead;
impl Skill for FsRead {
    fn name(&self) -> &'static str {
        "fs_read"
    }
    fn description(&self) -> &'static str {
        "Read a local file's text, confined to the configured [filesystem].roots. Output is \
        truncated to a character budget — pass a larger max_chars for more."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ReadArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ReadArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            let max = server.clamp_chars(args.max_chars);
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| invalid(format!("could not read '{}': {e}", path.display())))?;
            let text = String::from_utf8_lossy(&bytes);
            Ok(text_result(format!(
                "{}\n\n{}",
                path.display(),
                truncate_chars(&text, max)
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Read a file's text",
                args: r#"{"path": "Cargo.toml"}"#,
                note: Some("Output is truncated to the server's default character budget."),
            },
            SkillExample {
                title: "Read more of a long file",
                args: r#"{"path": "src/main.rs", "max_chars": 20000}"#,
                note: Some("Pass `max_chars` for more text than the default budget."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Inspect a known file's contents inside an allowed filesystem root.",
            "Read configuration / source / logs before editing them with `fs_edit`.",
            "Pull text into context for analysis without leaving the configured roots.",
        ]
    }
}

pub struct FsList;
impl Skill for FsList {
    fn name(&self) -> &'static str {
        "fs_list"
    }
    fn description(&self) -> &'static str {
        "List a directory's entries (name, type, size), confined to the configured roots. Omit \
        `path` to list a root."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArg>()?;
            let path = resolve(&server.fs, args.path.as_deref().unwrap_or("."))?;
            let mut rd = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| invalid(format!("could not list '{}': {e}", path.display())))?;
            let mut entries: Vec<(String, bool, u64)> = Vec::new();
            while let Some(e) = rd.next_entry().await.map_err(|e| internal(e.into()))? {
                let name = e.file_name().to_string_lossy().to_string();
                let md = e.metadata().await.ok();
                let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
                entries.push((name, is_dir, size));
            }
            entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let mut out = format!("{} ({} entries):\n", path.display(), entries.len());
            for (name, is_dir, size) in entries {
                if is_dir {
                    out.push_str(&format!("  {name}/\n"));
                } else {
                    out.push_str(&format!("  {name}  ({})\n", crate::util::human_size(size)));
                }
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "List the primary root",
                args: r#"{}"#,
                note: Some("Omitting `path` lists the first configured root."),
            },
            SkillExample {
                title: "List a subdirectory",
                args: r#"{"path": "src"}"#,
                note: Some("Directories show with a trailing `/`; files include a human size."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "See what's at a path before reading or editing files there.",
            "Get a quick directory inventory (names, types, sizes) without shelling out.",
            "Confirm a directory exists and is non-empty inside an allowed root.",
        ]
    }
}

pub struct FsStat;
impl Skill for FsStat {
    fn name(&self) -> &'static str {
        "fs_stat"
    }
    fn description(&self) -> &'static str {
        "Show a path's metadata (type, size, read-only, modified time), confined to the roots."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<StatArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            let md = tokio::fs::metadata(&path)
                .await
                .map_err(|e| invalid(format!("could not stat '{}': {e}", path.display())))?;
            let kind = if md.is_dir() {
                "directory"
            } else if md.is_file() {
                "file"
            } else {
                "other"
            };
            let mut out = format!(
                "{}\n  type: {kind}\n  size: {} ({} bytes)\n  read-only: {}",
                path.display(),
                crate::util::human_size(md.len()),
                md.len(),
                md.permissions().readonly()
            );
            if let Ok(modified) = md.modified() {
                if let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH) {
                    use chrono::{TimeZone, Utc};
                    if let Some(dt) = Utc.timestamp_opt(dur.as_secs() as i64, 0).single() {
                        out.push_str(&format!(
                            "\n  modified: {}",
                            dt.format("%Y-%m-%dT%H:%M:%SZ")
                        ));
                    }
                }
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Stat a file",
                args: r#"{"path": "Cargo.toml"}"#,
                note: Some("Returns type, size, read-only flag, and modified time."),
            },
            SkillExample {
                title: "Stat a directory",
                args: r#"{"path": "src"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check whether a path is a file or directory before acting on it.",
            "Confirm an edit landed by looking at the new modified time / size.",
            "Verify a path exists and is readable without pulling its contents.",
        ]
    }
}

pub struct FsFind;
impl Skill for FsFind {
    fn name(&self) -> &'static str {
        "fs_find"
    }
    fn description(&self) -> &'static str {
        "Find files under a directory by name pattern (`*` wildcard, or substring), confined to \
        the roots. Skips .git/target/node_modules; caps results."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FindArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<FindArgs>()?;
            let base = resolve(&server.fs, args.path.as_deref().unwrap_or("."))?;
            let pat = args.pattern.clone();
            let found = tokio::task::spawn_blocking(move || {
                let mut out: Vec<String> = Vec::new();
                let mut stack = vec![base.clone()];
                let mut visited = 0usize;
                while let Some(dir) = stack.pop() {
                    if out.len() >= 500 || visited > 50_000 {
                        break;
                    }
                    let Ok(rd) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for e in rd.flatten() {
                        visited += 1;
                        let p = e.path();
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        let rel = p
                            .strip_prefix(&base)
                            .unwrap_or(&p)
                            .to_string_lossy()
                            .to_string();
                        if is_dir {
                            let n = e.file_name();
                            let n = n.to_string_lossy();
                            if n == ".git" || n == "target" || n == "node_modules" {
                                continue;
                            }
                            stack.push(p);
                        } else if matches(&pat, &rel) {
                            out.push(rel);
                        }
                    }
                }
                out
            })
            .await
            .map_err(|e| internal(anyhow::anyhow!("find task failed: {e}")))?;
            if found.is_empty() {
                return Ok(text_result(format!("No files match: {}", args.pattern)));
            }
            let mut list = found;
            list.sort();
            Ok(text_result(format!(
                "{} match(es):\n{}",
                list.len(),
                list.iter()
                    .map(|p| format!("  {p}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Find every Rust source file under src/",
                args: r#"{"path": "src", "pattern": "*.rs"}"#,
                note: Some("Skips `.git`, `target`, and `node_modules`; caps at 500 hits."),
            },
            SkillExample {
                title: "Substring match anywhere under a root",
                args: r#"{"pattern": "config"}"#,
                note: Some("No `*` means substring match; `path` defaults to the primary root."),
            },
            SkillExample {
                title: "Wildcard with directory hint",
                args: r#"{"path": ".", "pattern": "src/*config*"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Locate files by name/extension before reading or editing them.",
            "Quick repository inventory (`*.rs`, `*.toml`, etc.) without `fd` / `find`.",
            "Find candidate config files when their exact path is unknown.",
        ]
    }
}

pub struct FsWrite;
impl Skill for FsWrite {
    fn name(&self) -> &'static str {
        "fs_write"
    }
    fn description(&self) -> &'static str {
        "Create or overwrite a file with the given content, confined to the roots. The parent \
        directory must already exist (use fs_mkdir first). Overwriting an existing file is \
        destructive: the first call to overwrite returns a confirmation token and does nothing — \
        call again with confirm=<token> to proceed (or confirm + trust=true to allow for the \
        session). Creating a new file does not require confirmation."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WriteArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<WriteArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            // Only guard the overwrite case — creating a new file is innocuous.
            // Use the blocking `std::fs::metadata` (cheap) under spawn_blocking
            // rather than `tokio::fs::try_exists`, which has surprised us on
            // Windows long-path (`\\?\`) inputs (returns false on a file that
            // exists). metadata().is_ok() is rock-solid here.
            let file_exists = {
                let p = path.clone();
                tokio::task::spawn_blocking(move || std::fs::metadata(&p).is_ok())
                    .await
                    .unwrap_or(false)
            };
            if file_exists {
                let bind = format!("fs_write:{}", path.display());
                let summary = format!(
                    "overwrite {} ({} bytes)",
                    path.display(),
                    args.content.len()
                );
                if let Decision::Challenge(msg) = server.guard.check(
                    &bind,
                    "fs_write",
                    server.fs.allow_destructive,
                    &summary,
                    args.confirm.as_deref(),
                    args.trust.unwrap_or(false),
                ) {
                    return Ok(text_result(msg));
                }
            }
            tokio::fs::write(&path, args.content.as_bytes())
                .await
                .map_err(|e| invalid(format!("could not write '{}': {e}", path.display())))?;
            Ok(text_result(format!(
                "Wrote {} bytes to {}",
                args.content.len(),
                path.display()
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Create a new file",
                args: r#"{"path": "notes/todo.md", "content": "- buy milk\n- write docs\n"}"#,
                note: Some(
                    "First-time create needs no confirmation; returns `Wrote N bytes to ...`.",
                ),
            },
            SkillExample {
                title: "Overwrite an existing file (first call, gets a token)",
                args: r#"{"path": "notes/todo.md", "content": "- updated\n"}"#,
                note: Some(
                    "Returns a confirmation prompt with a token because the file exists.",
                ),
            },
            SkillExample {
                title: "Overwrite — second call with the token",
                args: r#"{"path": "notes/todo.md", "content": "- updated\n", "confirm": "<token-from-prior-call>"}"#,
                note: Some(
                    "Add `trust: true` alongside `confirm` to skip the prompt for the rest of the session.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Create a brand-new file inside an allowed root with explicit contents.",
            "Replace a file wholesale when an in-place edit isn't the right shape.",
            "Materialize generated output (a report, a script, a config) to disk.",
        ]
    }
}

pub struct FsEdit;
impl Skill for FsEdit {
    fn name(&self) -> &'static str {
        "fs_edit"
    }
    fn description(&self) -> &'static str {
        "Edit a file by replacing `old_string` with `new_string`. By default `old_string` must \
        occur exactly once (set replace_all to replace every occurrence). Confined to the roots. \
        Destructive (mutates an existing file): the first call returns a confirmation token and \
        does nothing — call again with confirm=<token> to proceed (or confirm + trust=true to \
        allow for the session)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EditArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<EditArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            if args.old_string == args.new_string {
                return Err(invalid("old_string and new_string are identical"));
            }
            // Per-file guard binding so trusting one edit doesn't authorize
            // edits to other files in the same session.
            let bind = format!("fs_edit:{}", path.display());
            let summary = format!("edit {}", path.display());
            if let Decision::Challenge(msg) = server.guard.check(
                &bind,
                "fs_edit",
                server.fs.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| invalid(format!("could not read '{}': {e}", path.display())))?;
            let count = content.matches(&args.old_string).count();
            if count == 0 {
                return Err(invalid("old_string not found in the file"));
            }
            let replace_all = args.replace_all.unwrap_or(false);
            if count > 1 && !replace_all {
                return Err(invalid(format!(
                    "old_string occurs {count} times; pass replace_all=true or give a more specific string"
                )));
            }
            let updated = if replace_all {
                content.replace(&args.old_string, &args.new_string)
            } else {
                content.replacen(&args.old_string, &args.new_string, 1)
            };
            tokio::fs::write(&path, updated.as_bytes())
                .await
                .map_err(|e| invalid(format!("could not write '{}': {e}", path.display())))?;
            let n = if replace_all { count } else { 1 };
            Ok(text_result(format!(
                "Edited {} ({n} replacement{})",
                path.display(),
                if n == 1 { "" } else { "s" }
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Replace a unique snippet (first call gets a token)",
                args: r#"{"path": "src/main.rs", "old_string": "let x = 1;", "new_string": "let x = 2;"}"#,
                note: Some("First call returns a confirmation prompt with a token; nothing is written yet."),
            },
            SkillExample {
                title: "Apply the edit with the token",
                args: r#"{"path": "src/main.rs", "old_string": "let x = 1;", "new_string": "let x = 2;", "confirm": "<token-from-prior-call>"}"#,
                note: Some("Add `trust: true` to skip the prompt for further edits to THIS file."),
            },
            SkillExample {
                title: "Replace every occurrence",
                args: r#"{"path": "src/lib.rs", "old_string": "old_name", "new_string": "new_name", "replace_all": true, "confirm": "<token>"}"#,
                note: Some("Without `replace_all`, a non-unique match errors out."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Make a small, targeted edit to a file (rename a symbol, tweak a value).",
            "Apply a fix the model just derived without rewriting the whole file.",
            "Rename a token globally inside one file with `replace_all`.",
        ]
    }
}

pub struct FsMkdir;
impl Skill for FsMkdir {
    fn name(&self) -> &'static str {
        "fs_mkdir"
    }
    fn description(&self) -> &'static str {
        "Create a directory (and any missing parents), confined to the roots."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<StatArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            tokio::fs::create_dir_all(&path)
                .await
                .map_err(|e| invalid(format!("could not create '{}': {e}", path.display())))?;
            Ok(text_result(format!("Created directory {}", path.display())))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Create a single directory",
                args: r#"{"path": "build"}"#,
                note: None,
            },
            SkillExample {
                title: "Create nested directories",
                args: r#"{"path": "out/2026/reports"}"#,
                note: Some("Missing parents are created (mkdir -p semantics)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Prepare a target directory before writing files into it with `fs_write`.",
            "Materialize a nested output layout in one call instead of step-by-step.",
        ]
    }
}

pub struct FsDelete;
impl Skill for FsDelete {
    fn name(&self) -> &'static str {
        "fs_delete"
    }
    fn description(&self) -> &'static str {
        "Delete a file or directory, confined to the roots. Destructive: the first call returns a \
        confirmation token and does nothing — call again with confirm=<token> to proceed (or \
        confirm + trust=true to allow for the session). Pass recursive=true for a non-empty directory."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DeleteArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DeleteArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            let summary = format!("delete {}", path.display());
            if let Decision::Challenge(msg) = server.guard.check(
                "fs_delete",
                "fs_delete",
                server.fs.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let md = tokio::fs::metadata(&path)
                .await
                .map_err(|e| invalid(format!("could not stat '{}': {e}", path.display())))?;
            if md.is_dir() {
                let res = if args.recursive.unwrap_or(false) {
                    tokio::fs::remove_dir_all(&path).await
                } else {
                    tokio::fs::remove_dir(&path).await
                };
                res.map_err(|e| {
                    invalid(format!(
                        "could not remove directory '{}': {e} (pass recursive=true for non-empty)",
                        path.display()
                    ))
                })?;
            } else {
                tokio::fs::remove_file(&path)
                    .await
                    .map_err(|e| invalid(format!("could not remove '{}': {e}", path.display())))?;
            }
            Ok(text_result(format!("Deleted {}", path.display())))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Delete a single file (first call gets a token)",
                args: r#"{"path": "scratch/temp.log"}"#,
                note: Some(
                    "Destructive; first call returns a confirmation token and does nothing.",
                ),
            },
            SkillExample {
                title: "Delete with the token",
                args: r#"{"path": "scratch/temp.log", "confirm": "<token-from-prior-call>"}"#,
                note: None,
            },
            SkillExample {
                title: "Recursively remove a non-empty directory",
                args: r#"{"path": "build", "recursive": true, "confirm": "<token>"}"#,
                note: Some("Without `recursive: true`, removing a non-empty directory errors."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Clean up a generated artifact or scratch file inside an allowed root.",
            "Drop a build / cache directory wholesale with `recursive: true`.",
        ]
    }
}

pub struct FsMove;
impl Skill for FsMove {
    fn name(&self) -> &'static str {
        "fs_move"
    }
    fn description(&self) -> &'static str {
        "Move/rename a path. Both source and destination must be inside the roots. Destructive: the \
        first call returns a confirmation token and does nothing — call again with confirm=<token> \
        to proceed (or confirm + trust=true to allow for the session)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MoveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MoveArgs>()?;
            let source = resolve(&server.fs, &args.source)?;
            let dest = resolve(&server.fs, &args.dest)?;
            let summary = format!("move {} -> {}", source.display(), dest.display());
            if let Decision::Challenge(msg) = server.guard.check(
                "fs_move",
                "fs_move",
                server.fs.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            tokio::fs::rename(&source, &dest)
                .await
                .map_err(|e| invalid(format!("could not move '{}': {e}", source.display())))?;
            Ok(text_result(format!(
                "Moved {} -> {}",
                source.display(),
                dest.display()
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Rename a file (first call gets a token)",
                args: r#"{"source": "draft.md", "dest": "final.md"}"#,
                note: Some(
                    "Destructive; first call returns a confirmation token and does nothing.",
                ),
            },
            SkillExample {
                title: "Move with the token",
                args: r#"{"source": "draft.md", "dest": "docs/final.md", "confirm": "<token-from-prior-call>"}"#,
                note: Some("Both source and dest must be inside the configured roots."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Rename a file in place inside an allowed root.",
            "Relocate a file between directories without copy+delete.",
        ]
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(FsRead),
        Box::new(FsList),
        Box::new(FsStat),
        Box::new(FsFind),
        Box::new(FsWrite),
        Box::new(FsEdit),
        Box::new(FsMkdir),
        Box::new(FsDelete),
        Box::new(FsMove),
    ]
}

/// Family metadata for the filesystem skills. No host probe is needed —
/// the read/write surface is pure Rust on `tokio::fs`, gated only by the
/// configured roots — so the capability check is a constant `Ready`. We
/// still register the family so the dashboard's Tools page can render a
/// description and the canonical multi-step flow.
pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "filesystem"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Read and (with confirmation) edit files and directories on the local machine, \
         confined to the configured `[filesystem].roots`. Destructive ops route through \
         the confirmation guard unless `allow_destructive` is set."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        // Pure-Rust file I/O — no host binary required. Operator gating
        // happens via [filesystem].enabled in config.
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `fs_find { path: \"src\", pattern: \"*.rs\" }` to locate candidate files.\n\
             2. `fs_read { path: \"src/main.rs\" }` to inspect the suspect file.\n\
             3. `fs_edit { path: \"src/main.rs\", old_string: \"...\", new_string: \"...\" }` (confirm on second call) to apply the fix.\n\
             4. `fs_stat { path: \"src/main.rs\" }` to confirm the edit landed.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{confine, matches};
    use std::path::PathBuf;

    #[test]
    fn glob_and_substring_match() {
        assert!(matches("*.rs", "src/main.rs"));
        assert!(matches("src/*config*", "src/app_config.toml"));
        assert!(matches("main", "src/MAIN.rs")); // substring, case-insensitive
        assert!(!matches("*.toml", "src/main.rs"));
    }

    #[test]
    fn confine_rejects_parent_and_escapes() {
        let root = std::env::current_dir().unwrap().canonicalize().unwrap();
        let roots = vec![root.clone()];
        // A normal relative path resolves inside the root.
        let ok = confine(&roots, "Cargo.toml").unwrap();
        assert!(ok.starts_with(&root));
        // `..` is rejected outright.
        assert!(confine(&roots, "../etc/passwd").is_err());
        // An absolute path outside the root is rejected.
        let outside = if cfg!(windows) { "C:\\Windows" } else { "/etc" };
        assert!(confine(&roots, outside).is_err());
        let _ = PathBuf::new();
    }
}
