//! Jupyter notebook (`.ipynb`) skills — read and summarize notebooks without
//! executing them. Off by default (`[notebook].enabled`). Paths confined to
//! `[filesystem].roots`. A `.ipynb` file is a JSON document; we parse it with
//! `serde_json` and surface cells in a way the model can consume.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{fs_read_bytes, schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

fn load(server: &crate::Lodestone, path: &str) -> Result<(std::path::PathBuf, Value), McpError> {
    let (p, bytes) = fs_read_bytes(server, path)?;
    let json: Value =
        serde_json::from_slice(&bytes).map_err(|e| invalid(format!("not JSON: {e}")))?;
    Ok((p, json))
}

fn cell_source(cell: &Value) -> String {
    match cell.get("source") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .concat(),
        _ => String::new(),
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathArgs {
    /// Path to a `.ipynb` file.
    path: String,
}

pub struct NotebookInfo;
impl Skill for NotebookInfo {
    fn name(&self) -> &'static str {
        "notebook_info"
    }
    fn description(&self) -> &'static str {
        "Summarize a Jupyter notebook: kernel, language, notebook format version, cell counts \
        by type (code / markdown / raw), and any author/title metadata."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let (p, j) = load(server, &args.path)?;
            let meta = j.get("metadata").cloned().unwrap_or(Value::Null);
            let kernel = meta
                .get("kernelspec")
                .and_then(|k| k.get("display_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let lang = meta
                .get("language_info")
                .and_then(|l| l.get("name"))
                .and_then(|v| v.as_str())
                .or_else(|| {
                    meta.get("kernelspec")
                        .and_then(|k| k.get("language"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("?");
            let nb_ver = format!(
                "{}.{}",
                j.get("nbformat").and_then(|v| v.as_i64()).unwrap_or(0),
                j.get("nbformat_minor")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
            );
            let empty: Vec<Value> = Vec::new();
            let cells = j.get("cells").and_then(|v| v.as_array()).unwrap_or(&empty);
            let mut counts = (0u32, 0u32, 0u32); // code, markdown, raw
            for c in cells {
                match c.get("cell_type").and_then(|v| v.as_str()) {
                    Some("code") => counts.0 += 1,
                    Some("markdown") => counts.1 += 1,
                    Some("raw") => counts.2 += 1,
                    _ => {}
                }
            }
            Ok(text_result(format!(
                "{}\n  kernel: {kernel}\n  language: {lang}\n  format: {nb_ver}\n  cells: {} total ({} code, {} markdown, {} raw)",
                p.display(),
                cells.len(),
                counts.0,
                counts.1,
                counts.2,
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Summarize a Python notebook",
                args: r#"{"path": "notebooks/analysis.ipynb"}"#,
                note: Some("Returns kernel, language, nbformat version, and cell counts by type."),
            },
            SkillExample {
                title: "Notebook with a non-default kernel",
                args: r#"{"path": "notebooks/julia_work.ipynb"}"#,
                note: Some(
                    "Same output shape; kernel/language come from the notebook's metadata block.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decide whether a notebook is worth opening based on size and cell mix.",
            "Identify the runtime / kernel before suggesting a re-execution path.",
            "Quick header pass before drilling into specific cells via notebook_cells.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CellsArgs {
    /// Path to a `.ipynb` file inside an allowed root.
    path: String,
    /// Filter by cell type (`code`, `markdown`, `raw`). Default: all.
    #[serde(default)]
    cell_type: Option<String>,
    /// Skip this many matching cells (default 0).
    #[serde(default)]
    offset: Option<u32>,
    /// Max cells to return (default 20, capped at 200).
    #[serde(default)]
    max: Option<u32>,
    /// Per-cell max characters of source to include (default 1000, capped at 20000).
    #[serde(default)]
    max_chars: Option<u32>,
}

pub struct NotebookCells;
impl Skill for NotebookCells {
    fn name(&self) -> &'static str {
        "notebook_cells"
    }
    fn description(&self) -> &'static str {
        "List cells from a Jupyter notebook with their type and source (truncated). Use \
        `cell_type` to filter, `offset`/`max` to page, `max_chars` to control snippet size."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CellsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<CellsArgs>()?;
            let (p, j) = load(server, &args.path)?;
            let offset = args.offset.unwrap_or(0) as usize;
            let max = args.max.unwrap_or(20).clamp(1, 200) as usize;
            let max_chars = args.max_chars.unwrap_or(1000).clamp(1, 20_000) as usize;
            let want = args.cell_type.as_deref().map(|s| s.to_ascii_lowercase());
            let empty: Vec<Value> = Vec::new();
            let cells = j.get("cells").and_then(|v| v.as_array()).unwrap_or(&empty);
            let filtered: Vec<(usize, &Value)> = cells
                .iter()
                .enumerate()
                .filter(
                    |(_, c)| match (&want, c.get("cell_type").and_then(|v| v.as_str())) {
                        (Some(w), Some(t)) => t == w,
                        (None, _) => true,
                        _ => false,
                    },
                )
                .collect();
            let total = filtered.len();
            let shown: Vec<&(usize, &Value)> = filtered.iter().skip(offset).take(max).collect();
            let mut out = format!(
                "{}\n  cells matching: {} (showing {}–{})\n",
                p.display(),
                total,
                offset,
                offset + shown.len()
            );
            for (idx, c) in &shown {
                let ty = c.get("cell_type").and_then(|v| v.as_str()).unwrap_or("?");
                let src = cell_source(c);
                let preview: String = src.chars().take(max_chars).collect();
                let suffix = if src.chars().count() > max_chars {
                    " …"
                } else {
                    ""
                };
                out.push_str(&format!("\n── cell {idx} ({ty}) ──\n{preview}{suffix}\n"));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "First 20 cells, any type",
                args: r#"{"path": "notebooks/analysis.ipynb"}"#,
                note: Some("Defaults: 20 cells, 1000 chars per source preview."),
            },
            SkillExample {
                title: "Only code cells, paged",
                args: r#"{"path": "notebooks/analysis.ipynb", "cell_type": "code", "offset": 20, "max": 10}"#,
                note: Some("Use offset / max to page through long notebooks."),
            },
            SkillExample {
                title: "Bigger source snippets",
                args: r#"{"path": "notebooks/analysis.ipynb", "cell_type": "markdown", "max_chars": 5000}"#,
                note: Some("Raise max_chars when individual cells are documentation-heavy."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Read a notebook's cell contents without launching Jupyter.",
            "Extract all markdown narration for a write-up.",
            "Diff or audit notebook code cells against expected steps.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(NotebookInfo), Box::new(NotebookCells)]
}
