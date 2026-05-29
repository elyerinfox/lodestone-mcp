# Shell execution — `shell_run`

|  |  |
| --- | --- |
| **Module** | [`src/skills/shell.rs`](../../src/skills/shell.rs) |
| **Tools** | `shell_run` |
| **Network** | local-only (shell) |
| **Default** | **off** — gated by `[shell]` (`enabled`) |
| **Config** | `[shell]` in [`config/11-shell.toml`](../../config/11-shell.toml) |

## What it does
Runs commands on the machine the server runs on — arbitrary code execution, the most
dangerous capability, so it is **off by default** and must be explicitly granted
(`[shell].enabled = true`). It runs with the server's own privileges. There are two
modes: an **allowlist** mode (the default) where only programs named in
`[shell].allow` may run — executed directly with no shell, so `;`, `|`, `$(…)` are
inert literals — and an **unrestricted** mode (`allow_unrestricted = true`) where the
whole command line runs through the system shell (`sh -c` / `cmd /C`): full power,
full risk. Every run has a timeout (the process is killed) and a working directory.

**Confirmation.** A shell command is arbitrary code, so **every** `shell_run` goes
through the confirmation guard (golden rule 8): the first call returns a one-time
token and runs nothing; call again with `confirm=<token>` to execute, or `confirm`
+ `trust=true` to stop prompting for **that exact command** for the session.
`[shell].allow_destructive = true` pre-authorizes and skips the prompt (not
recommended).

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `shell_run` | `command`, `workdir?`, `timeout_secs?`, `confirm?`, `trust?` | **destructive / arbitrary exec** | Run a command (confirm first); returns exit code + stdout/stderr (output truncated to the server char budget). |

`workdir` defaults to `[shell].workdir` (else the server CWD); `timeout_secs`
defaults to `[shell].timeout_secs` and is clamped to 1–600.

## Configuration & gating
The tool is hidden unless `[shell].enabled = true`, and even then every call confirms
at call time (see above). Keys: `enabled` (expose the tool), `allow` (allowlisted
program names — matched on the command's first token, by basename, case-insensitively;
empty + not unrestricted = nothing runs), `allow_unrestricted` (run anything via the
system shell — full RCE), `allow_destructive` (skip the per-command prompt),
`timeout_secs`, and `workdir`. Env: `LODESTONE_SHELL_ENABLED`,
`LODESTONE_SHELL_ALLOW_UNRESTRICTED`, `LODESTONE_SHELL_ALLOW_DESTRUCTIVE` (others
follow the `LODESTONE_SHELL_*` pattern). Prefer allowlist mode with a tight `allow`
list; only set `allow_unrestricted` when you fully trust the calling model and host.

## Example uses
- **Run a build step (allowlisted)** — with `allow = ["cargo"]`, `shell_run` (`command="cargo build --release"`) executes `cargo` directly.
- **Scoped invocation** — `shell_run` (`command="pytest -q", workdir="/repo", timeout_secs=120`) runs tests in a specific dir with a longer timeout.
- **Pipelines (unrestricted only)** — with `allow_unrestricted = true`, `shell_run` (`command="ls | wc -l"`) runs the whole line through the shell; in allowlist mode `|` would be a literal arg.

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
