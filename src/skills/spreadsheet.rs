//! Spreadsheet skill — read, query, and write tabular data. **Off by default**
//! (`[spreadsheet].enabled`). Reads CSV/TSV (via `csv`) and XLSX/XLS/ODS (via
//! `calamine`); writes CSV or XLSX (via `rust_xlsxwriter`). Every path is confined
//! to `[filesystem].roots` (same rules as the filesystem skill), and `sheet_write`
//! goes through the confirmation [`guard`](crate::skills::guard). The blocking
//! parse/serialize work runs on `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::filesystem::resolve;
use crate::skills::guard::Decision;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// Cap on rows/cols pulled into a single response so a huge sheet can't blow up output.
const MAX_ROWS: usize = 1000;
const MAX_COLS: usize = 64;

/// True for an extension we read via calamine (spreadsheet workbooks).
fn is_workbook(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("xlsx" | "xlsm" | "xlsb" | "xls" | "ods")
    )
}

fn is_tsv(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("tsv"))
}

/// Render one calamine cell as a string.
fn cell_str(d: &calamine::Data) -> String {
    use calamine::Data::*;
    match d {
        Int(i) => i.to_string(),
        Float(f) => {
            let s = format!("{f:.6}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        String(s) => s.clone(),
        Bool(b) => b.to_string(),
        DateTime(dt) => dt.as_f64().to_string(),
        DateTimeIso(s) => s.clone(),
        DurationIso(s) => s.clone(),
        Error(e) => format!("#ERR({e:?})"),
        Empty => std::string::String::new(),
    }
}

/// Load a file (CSV/TSV/workbook) into a row grid (capped). For workbooks, `sheet`
/// selects a named sheet (default: the first). Blocking — call via spawn_blocking.
fn load(path: &Path, sheet: Option<&str>) -> Result<Vec<Vec<String>>> {
    if is_workbook(path) {
        use calamine::Reader;
        let mut wb = calamine::open_workbook_auto(path)
            .with_context(|| format!("opening workbook '{}'", path.display()))?;
        let names = wb.sheet_names();
        let name = match sheet {
            Some(s) => s.to_string(),
            None => names
                .first()
                .cloned()
                .ok_or_else(|| anyhow!("workbook has no sheets"))?,
        };
        let range = wb
            .worksheet_range(&name)
            .with_context(|| format!("reading sheet '{name}'"))?;
        let mut out = Vec::new();
        for row in range.rows().take(MAX_ROWS) {
            out.push(row.iter().take(MAX_COLS).map(cell_str).collect());
        }
        Ok(out)
    } else {
        let delim = if is_tsv(path) { b'\t' } else { b',' };
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(delim)
            .has_headers(false)
            .flexible(true)
            .from_path(path)
            .with_context(|| format!("opening '{}'", path.display()))?;
        let mut out = Vec::new();
        for rec in rdr.records().take(MAX_ROWS) {
            let rec = rec?;
            out.push(rec.iter().take(MAX_COLS).map(|s| s.to_string()).collect());
        }
        Ok(out)
    }
}

/// Format a row grid as an aligned text table.
fn render(rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return "(empty)".to_string();
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; cols];
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            widths[i] = widths[i].max(c.chars().count().min(40));
        }
    }
    let mut out = String::new();
    for r in rows {
        let mut line = String::new();
        for (i, w) in widths.iter().enumerate() {
            let cell = r.get(i).map(String::as_str).unwrap_or("");
            let cell: String = cell.chars().take(40).collect();
            line.push_str(&format!("{cell:<width$}  ", width = *w));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end().to_string()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadArgs {
    /// Path to a CSV/TSV/XLSX/XLS/ODS file (confined to [filesystem].roots).
    path: String,
    /// For workbooks, the sheet name to read (default: the first sheet).
    #[serde(default)]
    sheet: Option<String>,
    /// Max rows to return (default 50, capped at 1000).
    #[serde(default)]
    max_rows: Option<usize>,
}

pub struct SheetRead;
impl Skill for SheetRead {
    fn name(&self) -> &'static str {
        "sheet_read"
    }
    fn description(&self) -> &'static str {
        "Read tabular data from a local CSV/TSV or XLSX/XLS/ODS file (off by default; \
        [spreadsheet]). Returns rows as an aligned text table. Confined to [filesystem].roots; \
        pick a `sheet` for multi-sheet workbooks."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ReadArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ReadArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            let sheet = args.sheet.clone();
            let limit = args.max_rows.unwrap_or(50).clamp(1, MAX_ROWS);
            let rows = load_blocking(path, sheet).await?;
            let shown: Vec<Vec<String>> = rows.iter().take(limit).cloned().collect();
            let mut body = render(&shown);
            if rows.len() > limit {
                body.push_str(&format!("\n… ({} more rows)", rows.len() - limit));
            }
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Read a CSV",
                args: r#"{"path": "data/people.csv"}"#,
                note: Some("Returns rows as an aligned text table; defaults to 50 rows."),
            },
            SkillExample {
                title: "Read a specific sheet from an XLSX",
                args: r#"{"path": "data/report.xlsx", "sheet": "Q3", "max_rows": 200}"#,
                note: Some("`sheet` is required when the workbook has multiple sheets and you want a non-default one."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Peek at a CSV / TSV / XLSX file's contents before analyzing it.",
            "Pull a small slice of a sheet into context as a text table.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryArgs {
    /// Path to a CSV/TSV/XLSX/XLS/ODS file (confined to [filesystem].roots).
    path: String,
    /// For workbooks, the sheet name (default: the first sheet).
    #[serde(default)]
    sheet: Option<String>,
    /// Header column name to filter on (the first row is treated as the header).
    column: String,
    /// Keep rows whose `column` value equals this (case-insensitive, trimmed).
    equals: String,
    /// Optional list of header names to project (default: all columns).
    #[serde(default)]
    select: Option<Vec<String>>,
    /// Max matching rows to return (default 50, capped at 1000).
    #[serde(default)]
    max_rows: Option<usize>,
}

pub struct SheetQuery;
impl Skill for SheetQuery {
    fn name(&self) -> &'static str {
        "sheet_query"
    }
    fn description(&self) -> &'static str {
        "Filter and project rows of a local CSV/XLSX file by header name (off by default; \
        [spreadsheet]). Treats the first row as the header; keeps rows where `column` equals \
        `equals` (case-insensitive), and returns `select` columns (or all). Confined to \
        [filesystem].roots."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<QueryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<QueryArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            let limit = args.max_rows.unwrap_or(50).clamp(1, MAX_ROWS);
            let rows = load_blocking(path, args.sheet.clone()).await?;
            let result = query(
                &rows,
                &args.column,
                &args.equals,
                args.select.as_deref(),
                limit,
            )
            .map_err(invalid)?;
            Ok(text_result(result))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Filter all rows where city = NYC",
                args: r#"{"path": "data/people.csv", "column": "city", "equals": "NYC"}"#,
                note: Some(
                    "Match is case-insensitive and trimmed; first row is treated as the header.",
                ),
            },
            SkillExample {
                title: "Filter and project specific columns",
                args: r#"{"path": "data/people.csv", "column": "city", "equals": "NYC", "select": ["name", "age"]}"#,
                note: Some(
                    "Output table contains only the projected columns plus the matching count.",
                ),
            },
            SkillExample {
                title: "Filter a named XLSX sheet",
                args: r#"{"path": "data/report.xlsx", "sheet": "Sales", "column": "region", "equals": "EMEA", "max_rows": 200}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pick rows out of a CSV / XLSX by a single column equality without `sheet_read` + slicing.",
            "Project to just the columns the model cares about for downstream analysis.",
            "Quick lookups in a tabular file the user pointed at.",
        ]
    }
}

/// Run a header-based filter+projection over a loaded grid.
fn query(
    rows: &[Vec<String>],
    column: &str,
    equals: &str,
    select: Option<&[String]>,
    limit: usize,
) -> Result<String, String> {
    let header = rows.first().ok_or("file is empty (no header row)")?;
    let col_idx = header
        .iter()
        .position(|h| h.eq_ignore_ascii_case(column))
        .ok_or_else(|| format!("no column named '{column}' in header"))?;
    // Resolve projected columns to indices (default: all).
    let proj: Vec<usize> = match select {
        Some(names) => {
            let mut idxs = Vec::new();
            for n in names {
                let i = header
                    .iter()
                    .position(|h| h.eq_ignore_ascii_case(n))
                    .ok_or_else(|| format!("no column named '{n}' in header"))?;
                idxs.push(i);
            }
            idxs
        }
        None => (0..header.len()).collect(),
    };
    let want = equals.trim();
    let mut out: Vec<Vec<String>> = Vec::new();
    // Always include the (projected) header.
    out.push(proj.iter().map(|&i| header[i].clone()).collect());
    let mut matched = 0;
    for row in rows.iter().skip(1) {
        let cell = row.get(col_idx).map(String::as_str).unwrap_or("");
        if cell.trim().eq_ignore_ascii_case(want) {
            out.push(
                proj.iter()
                    .map(|&i| row.get(i).cloned().unwrap_or_default())
                    .collect(),
            );
            matched += 1;
            if matched >= limit {
                break;
            }
        }
    }
    let mut body = render(&out);
    body.push_str(&format!("\n\n{matched} matching row(s)."));
    Ok(body)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WriteArgs {
    /// Destination path (confined to [filesystem].roots). `.csv`/`.tsv` writes a
    /// delimited file; `.xlsx` writes a workbook.
    path: String,
    /// The rows to write (each an array of cell strings); the first row is usually
    /// the header.
    rows: Vec<Vec<String>>,
    /// For an `.xlsx` output, the worksheet name (default: "Sheet1").
    #[serde(default)]
    sheet_name: Option<String>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for sheet_write for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct SheetWrite;
impl Skill for SheetWrite {
    fn name(&self) -> &'static str {
        "sheet_write"
    }
    fn description(&self) -> &'static str {
        "Write rows to a local CSV/TSV or XLSX file (off by default; [spreadsheet]). Writes a file, \
        so the first call returns a confirmation token and does nothing; call again with \
        confirm=<token> (or confirm + trust=true). Confined to [filesystem].roots; the format is \
        chosen by the path's extension."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WriteArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<WriteArgs>()?;
            let path = resolve(&server.fs, &args.path)?;
            let summary = format!("write {} row(s) to {}", args.rows.len(), args.path);
            if let Decision::Challenge(msg) = server.guard.check(
                "sheet_write",
                "sheet_write",
                server.cfg.spreadsheet.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let n = args.rows.len();
            let sheet_name = args.sheet_name.unwrap_or_else(|| "Sheet1".to_string());
            write_blocking(path, args.rows, sheet_name).await?;
            Ok(text_result(format!("Wrote {n} row(s) to {}.", args.path)))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Write a CSV (first call gets a token)",
                args: r#"{"path": "out/people.csv", "rows": [["name", "city"], ["alice", "nyc"], ["bob", "sf"]]}"#,
                note: Some("Destructive (writes a file); first call returns a confirmation token."),
            },
            SkillExample {
                title: "Write with the token",
                args: r#"{"path": "out/people.csv", "rows": [["name", "city"], ["alice", "nyc"]], "confirm": "<token-from-prior-call>"}"#,
                note: None,
            },
            SkillExample {
                title: "Write an XLSX with a named sheet",
                args: r#"{"path": "out/report.xlsx", "rows": [["q", "rev"], ["Q1", "120"]], "sheet_name": "Sales", "confirm": "<token>"}"#,
                note: Some("Format is chosen by the path's extension (`.csv` / `.tsv` / `.xlsx`)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Emit a small computed table to disk as CSV for downstream tools.",
            "Generate a single-sheet XLSX report from rows assembled in-context.",
        ]
    }
}

/// Serialize a grid to disk (CSV/TSV/XLSX by extension). Blocking — spawn_blocking.
fn write_grid(path: &Path, rows: &[Vec<String>], sheet_name: &str) -> Result<()> {
    if is_workbook(path) {
        let mut wb = rust_xlsxwriter::Workbook::new();
        let ws = wb.add_worksheet();
        ws.set_name(sheet_name).ok();
        for (r, row) in rows.iter().enumerate() {
            for (c, val) in row.iter().enumerate() {
                ws.write_string(r as u32, c as u16, val)
                    .map_err(|e| anyhow!("xlsx write error at r{r}c{c}: {e}"))?;
            }
        }
        wb.save(path)
            .with_context(|| format!("saving workbook '{}'", path.display()))?;
    } else {
        let delim = if is_tsv(path) { b'\t' } else { b',' };
        let mut wtr = csv::WriterBuilder::new()
            .delimiter(delim)
            .from_path(path)
            .with_context(|| format!("creating '{}'", path.display()))?;
        for row in rows {
            wtr.write_record(row)?;
        }
        wtr.flush()?;
    }
    Ok(())
}

/// `load` on a blocking thread, mapped to MCP errors.
async fn load_blocking(path: PathBuf, sheet: Option<String>) -> Result<Vec<Vec<String>>, McpError> {
    tokio::task::spawn_blocking(move || load(&path, sheet.as_deref()))
        .await
        .map_err(|e| internal(anyhow!("task join error: {e}")))?
        .map_err(internal)
}

/// `write_grid` on a blocking thread, mapped to MCP errors.
async fn write_blocking(
    path: PathBuf,
    rows: Vec<Vec<String>>,
    sheet: String,
) -> Result<(), McpError> {
    tokio::task::spawn_blocking(move || write_grid(&path, &rows, &sheet))
        .await
        .map_err(|e| internal(anyhow!("task join error: {e}")))?
        .map_err(internal)
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SheetRead),
        Box::new(SheetQuery),
        Box::new(SheetWrite),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<Vec<String>> {
        vec![
            vec!["name".into(), "city".into(), "age".into()],
            vec!["alice".into(), "nyc".into(), "30".into()],
            vec!["bob".into(), "sf".into(), "25".into()],
            vec!["carol".into(), "nyc".into(), "40".into()],
        ]
    }

    #[test]
    fn query_filters_and_projects() {
        let g = grid();
        let out = query(&g, "city", "NYC", Some(&["name".into(), "age".into()]), 100).unwrap();
        assert!(out.contains("alice"));
        assert!(out.contains("carol"));
        assert!(!out.contains("bob")); // sf filtered out
        assert!(!out.contains("sf"));
        assert!(out.contains("2 matching"));
        // Projected header present, dropped column absent.
        assert!(out.contains("name"));
        assert!(!out.contains("city"));
    }

    #[test]
    fn query_unknown_column_errors() {
        let g = grid();
        assert!(query(&g, "nope", "x", None, 10).is_err());
        assert!(query(&g, "city", "nyc", Some(&["missing".into()]), 10).is_err());
    }

    #[test]
    fn csv_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lode-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.csv");
        write_grid(&path, &grid(), "Sheet1").unwrap();
        let back = load(&path, None).unwrap();
        assert_eq!(back, grid());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn xlsx_roundtrip() {
        let dir = std::env::temp_dir().join(format!("lode-sheet-x-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.xlsx");
        write_grid(&path, &grid(), "Data").unwrap();
        let back = load(&path, None).unwrap();
        assert_eq!(back[0], vec!["name", "city", "age"]);
        assert_eq!(back[1][0], "alice");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
