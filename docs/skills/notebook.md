# Jupyter notebook reader — `notebook_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/notebook.rs`](../../src/skills/notebook.rs) |
| **Tools** | `notebook_info`, `notebook_cells` |
| **Network** | none |
| **Default** | **off** — gated by `[notebook]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[notebook].enabled` via `LODESTONE_NOTEBOOK_ENABLED`. Paths confined to `[filesystem].roots`. Defaults in [`src/config.rs`](../../src/config.rs). |

## What it does

Read a Jupyter `.ipynb` file. No execution — just parse the JSON and surface
cells in a readable form (markdown + code + optional outputs).

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `notebook_info` | `path` | Summary: kernel, language, cell count by type. |
| `notebook_cells` | `path`, `max?`, `include_outputs?` | Walk cells (default first 20). Optionally include execution outputs (text / errors / data summaries). |

## Example uses

- **What's in this notebook?** — `notebook_info { path: "analysis.ipynb" }`.
- **Read the analysis** — `notebook_cells { path: "analysis.ipynb", include_outputs: true }`.

## Notes

- **Read-only.** No kernel start / cell execution.
- **Outputs are summarized** — large dataframes / images are not embedded
  verbatim.

## See also

- [tools.md](../tools.md)
- [skills/python.md](python.md) — actually run a Python snippet.
