# Local filesystem — `fs_read` / `fs_list` / `fs_stat` / `fs_find` / `fs_write` / `fs_edit` / `fs_mkdir` / `fs_delete` / `fs_move`

|  |  |
| --- | --- |
| **Module** | [`src/skills/filesystem.rs`](../../src/skills/filesystem.rs) |
| **Tools** | `fs_read`, `fs_list`, `fs_stat`, `fs_find`, `fs_write`, `fs_edit`, `fs_mkdir`, `fs_delete`, `fs_move` |
| **Network** | local-only (filesystem) |
| **Default** | **off** — gated by `[filesystem]` (`enabled`) |
| **Config** | `[filesystem]` in [`config/10-filesystem.toml`](../../config/10-filesystem.toml) |

## What it does
Reads, searches, and edits files and directories on the machine the server runs on.
A powerful, dangerous capability, so it is **off by default** — the user must
explicitly grant it (`[filesystem].enabled = true`). Every path is **confined** to
the configured `[filesystem].roots` (default: the server's working directory): `..`
components are rejected and symlinks are resolved (`canonicalize`), so no operation
can escape a root. The destructive tools (`fs_delete`, `fs_move`) never run on the
first call — they go through the confirmation guard.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `fs_read` | `path`, `max_chars?` | read | Read a file's text (truncated to a character budget; raise `max_chars` for more). |
| `fs_list` | `path?` | read | List a directory's entries (name, type, size); omit `path` to list a root. |
| `fs_stat` | `path` | read | A path's type, size, read-only flag, and modified time. |
| `fs_find` | `pattern`, `path?` | read | Find files by name — `*` wildcard or substring; skips `.git`/`target`/`node_modules`, caps results. |
| `fs_write` | `path`, `content`, `confirm?`, `trust?` | **destructive** | Create a file (no confirm) or overwrite an existing one (confirm first). |
| `fs_edit` | `path`, `old_string`, `new_string`, `replace_all?`, `confirm?`, `trust?` | **destructive** | Replace text; `old_string` must occur once unless `replace_all=true` (confirm first). |
| `fs_mkdir` | `path` | write | Create a directory (with any missing parents). |
| `fs_delete` | `path`, `recursive?`, `confirm?`, `trust?` | **destructive** | Delete a file/directory (confirm first; `recursive=true` for a non-empty dir). |
| `fs_move` | `source`, `dest`, `confirm?`, `trust?` | **destructive** | Move/rename a path (confirm first; overwrites an existing dest). |

## Configuration & gating
The whole family is hidden unless `[filesystem].enabled = true` — there is no keyless
default; granting file access is a deliberate opt-in. Keys: `enabled` (expose the
tools), `roots` (allowed base dirs; empty = the working directory only), and
`allow_destructive` (pre-authorize the destructive tools). Env equivalents:
`LODESTONE_FS_ENABLED`, `LODESTONE_FS_ROOTS`, `LODESTONE_FS_ALLOW_DESTRUCTIVE`.

When enabled, `fs_write` (overwrite only — creating a new file is innocuous),
`fs_edit`, `fs_delete`, and `fs_move` are still **exposed and gated at call time**
by the confirmation [`guard`](../../src/skills/guard.rs) (golden rule 8). Bindings
are **per-path** (e.g. `fs_write:/foo/bar.txt`), so `trust=true` only whitelists
that one path — a later overwrite of a different file still challenges. The first
call performs nothing and returns a one-time `confirm` token describing exactly
what will happen; call again with `confirm=<token>` to execute, or
`confirm=<token>, trust=true` to also stop being asked for that specific path
for the rest of the session. Tokens are single-use and expire after 5 minutes.
Setting `[filesystem].allow_destructive = true` pre-authorizes all four and
skips the prompt entirely. Scope `roots` narrowly and run behind a host that
approves tool calls.

## Example uses
- **Read a config value** — `fs_find` (`pattern="*config*.toml"`) → `fs_read` the hit → report the value.
- **Apply a small fix** — `fs_find` → `fs_read` to locate the snippet → `fs_edit` with a unique `old_string`/`new_string`.
- **Scaffold a directory** — `fs_mkdir` (`path="src/new"`) → `fs_write` the file into it.
- **Clean up safely** — `fs_delete` (`path="tmp", recursive=true`) returns a token; call again with `confirm=<token>` to remove it.

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
