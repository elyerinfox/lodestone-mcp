# Python runner — `python_run`

|  |  |
| --- | --- |
| **Module** | [`src/skills/python.rs`](../../src/skills/python.rs) |
| **Tools** | `python_run` |
| **Network** | only what the script itself reaches |
| **Default** | **off** — gated by `[python]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[python].enabled` / `interpreter` / `timeout_secs` / `allow_destructive` via `LODESTONE_PYTHON_*`. Defaults in [`src/config.rs`](../../src/config.rs). |

## What it does

Execute a Python snippet via the configured interpreter (default `python3` on
Unix, `python` on Windows). **Destructive — guarded.** Off by default because
arbitrary Python is, by definition, arbitrary code; the per-call timeout caps
runaway scripts.

## Tool

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `python_run` | `code`, `stdin?`, `args?`, `timeout_secs?`, `confirm?`, `trust?` | Run a Python script. Stdin is fed verbatim; `args` is pre-split (no shell). Returns stdout, stderr, and exit code. |

## Example uses

- **Quick calculation** —
  `python_run { code: "import statistics; print(statistics.median([1,5,3,8,2]))" }`.
- **JSON transform** —
  `python_run { code: "import json,sys; d=json.load(sys.stdin); print(json.dumps([x['name'] for x in d]))", stdin: "<json>" }`.
- **Long-running task** — bump `timeout_secs` (default 30, capped at 600).

## Notes

- **Destructive — guarded** (golden rule 8). First call returns a `confirm`
  token; second call with that token executes. `[python].allow_destructive =
  true` pre-authorizes.
- **No package install.** The interpreter runs as-is; if `numpy` isn't
  installed system-wide, your script can't import it. Use a venv interpreter
  if you need packages.
- **`args` is pre-split.** No shell escaping — args reach `sys.argv` verbatim.

## See also

- [tools.md](../tools.md)
- [skills/shell.md](shell.md) — arbitrary shell, even more dangerous.
- [skills/databases.md](databases.md) — for SQL, prefer this over piping into
  Python.
