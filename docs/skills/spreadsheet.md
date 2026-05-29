# Spreadsheets — `sheet_read`, `sheet_query`, `sheet_write`

|  |  |
| --- | --- |
| **Module** | [`src/skills/spreadsheet.rs`](../../src/skills/spreadsheet.rs) |
| **Tools** | `sheet_read`, `sheet_query`, `sheet_write` |
| **Network** | none (local files) |
| **Default** | **off** — gated by `[spreadsheet]` |
| **Config** | `[spreadsheet]` in [`config/10-filesystem.toml`](../../config/10-filesystem.toml) |

## What it does
Reads, filters, and writes tabular data on disk. CSV/TSV via the `csv` crate;
XLSX/XLS/ODS reads via `calamine`; XLSX writes via `rust_xlsxwriter`. Off by
default. Every path is **confined to `[filesystem].roots`** (same `..`/symlink
rules as the filesystem skill), and `sheet_write` is routed through the
confirmation [guard](../golden-rules.md). Parsing/serialization runs off the async
runtime (`spawn_blocking`).

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `sheet_read` | `path`, `sheet?`, `max_rows?` | read | Read a sheet as an aligned text table. |
| `sheet_query` | `path`, `column`, `equals`, `sheet?`, `select?`, `max_rows?` | read | Keep rows where a header `column` equals a value; project `select` columns. |
| `sheet_write` | `path`, `rows`, `sheet_name?`, `confirm?`, `trust?` | **write** | Write rows to CSV/TSV or XLSX (format by extension). |

- **Format** is chosen by the file extension: `.csv`/`.tsv` (delimited), or
  `.xlsx`/`.xls`/`.xlsb`/`.ods` (workbook; writes are `.xlsx` only).
- `sheet_query` treats the **first row as the header**; `column`/`select` name
  header columns (case-insensitive); the match on `equals` is case-insensitive and
  trimmed.
- Reads are capped (1000 rows × 64 cols per response) so a huge sheet can't blow up
  output; `max_rows` narrows further.

## Example uses
- **Peek at a file** — `sheet_read { path: "data/users.xlsx", sheet: "Q1" }`.
- **Filter** — `sheet_query { path: "users.csv", column: "city", equals: "NYC",
  select: ["name", "email"] }`.
- **Export** — `sheet_write { path: "out.xlsx", rows: [["name","age"],["alice","30"]] }`
  → returns a confirm token; repeat with `confirm`.

## See also
[tools.md](../tools.md)
