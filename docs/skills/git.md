# Git CLI — `git_run`

|  |  |
| --- | --- |
| **Module** | [`src/skills/git.rs`](../../src/skills/git.rs) |
| **Tools** | `git_run` |
| **Network** | local-only (git) |
| **Default** | on — gated by `[git]` (`enabled`) |
| **Config** | `[git]` in [`config/12-git.toml`](../../config/12-git.toml) |

## What it does
Runs the local `git` binary against a repository working directory. Arguments are
passed directly to `git` (no shell, so they aren't re-interpreted) — you give them
without the leading `git`, e.g. `status -sb`, `log --oneline -10`, `commit -m "msg"`.
On by default; requires `git` on PATH (a clear "not found" message otherwise).
Most subcommands run freely, but **destructive subcommands** — `push`, `reset`,
`clean`, `rebase`, `filter-branch`, `filter-repo`, `gc`, `prune`, `reflog` — go
through the confirmation guard before they act.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `git_run` | `args`, `repo?`, `confirm?`, `trust?` | read / write / **destructive** | Run `git <args>`; returns exit code + stdout/stderr. Read/safe-write subcommands run immediately; destructive ones confirm first. |

`repo` defaults to `[git].repo` (else the server CWD); the per-command timeout comes
from `[git].timeout_secs` (clamped to 1–600).

## Configuration & gating
The tool is hidden when `[git].enabled = false`. Keys: `enabled`, `repo` (default
working directory), `timeout_secs`, and `allow_destructive` (pre-authorize the
destructive subcommands). Env equivalents follow the `LODESTONE_GIT_*` pattern.

The first non-flag token is the subcommand. When it is one of the destructive set,
the [`guard`](../../src/skills/guard.rs) (golden rule 8) returns a one-time `confirm`
token and does nothing; call again with `confirm=<token>` to execute, or
`confirm=<token>, trust=true` to stop being asked for **that subcommand** (keyed
`git:<subcmd>`) for the rest of the session. Tokens are single-use and expire after
5 minutes. Setting `[git].allow_destructive = true` pre-authorizes all of them and
skips the prompt.

## Example uses
- **Inspect a repo** — `git_run` (`args="status -sb"`) then `git_run` (`args="log --oneline -10"`) to see recent history.
- **Commit work** — `git_run` (`args="add -A"`) → `git_run` (`args='commit -m \"fix: …\"'`) (neither is destructive, so both run immediately).
- **Push with confirmation** — `git_run` (`args="push origin main"`) returns a token; call again with `confirm=<token>` to actually push.
- **Target another checkout** — `git_run` (`args="diff", repo="/path/to/other-repo"`).

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
