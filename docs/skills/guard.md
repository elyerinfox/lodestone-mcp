# Destructive-action guard (infrastructure, not a tool)

|  |  |
| --- | --- |
| **Module** | [`src/skills/guard.rs`](../../src/skills/guard.rs) |
| **Tools** | none directly — guards `fs_delete`, `fs_move`, `fs_write`, `fs_edit`, `fs_mkdir`, `docker_stop`, `docker_remove`, `docker_exec`, `docker_rmi`, `k8s_delete`, write-mode `db_query` / `redis_command`, `shell_run`, `python_run`, `ffmpeg_convert`, `sheet_write`, `systemd_start` / `_stop` / `_restart`, `memory_forget`, `solution_forget`, `conversation_forget`, `conversation_prune`, every destructive `git_run` subcommand |
| **Network** | none — process-local in-memory state |
| **Default** | always armed; per-family `[<family>].allow_destructive` pre-authorizes |

## What it does

A **client-agnostic** alternative to MCP elicitation (which not every
client supports — LM Studio doesn't, for example). Every destructive
tool calls `Guard::check` before acting. The result is one of:

- **Proceed.** Either the action is whitelisted for the rest of this
  session (`trust: true` from a prior call) OR the family has
  `allow_destructive = true` in config. The tool runs.
- **Challenge.** Returns a one-time `confirm` token plus a
  human-readable description of what *would* happen. The tool does
  nothing. The model has to call the same tool a second time with
  `confirm: "<token>"` to actually run it.

This is golden rule 8 in code form. A destructive op can never fire
in a single un-surfaced step.

## The two-call dance

```
1. Model calls:    fs_delete { path: "build.log" }
   Server returns: "Confirm to delete `build.log` — pass
                    confirm=\"a1b2c3…\" to run, or trust=true
                    to whitelist for this session."
2. Model calls:    fs_delete { path: "build.log",
                                confirm: "a1b2c3…" }
   Server returns: "Deleted `build.log`."
```

Adding `trust: true` to the second call whitelists this exact tool
(or this exact path, depending on the operation's grain) for the rest
of the process — subsequent matching calls skip the challenge. Trust
is in-memory: a restart clears it.

## Token lifetime

Tokens expire after **5 minutes** (`TOKEN_TTL`). A stale token gets
rejected with a fresh challenge. Tokens are bound to the exact
arguments the first call passed, so a leaked token can't be reused on
a different path.

## Pre-authorization via config

Each family with destructive surface carries an `allow_destructive`
flag:

```toml
[filesystem]
enabled = true
allow_destructive = true     # skip the guard for fs_delete / fs_move / ...

[docker]
enabled = true
allow_destructive = false    # docker_stop / _remove / _exec / _rmi still go through the guard
```

Pre-authorization is per-family on purpose — an operator can trust
the filesystem tools (because the roots are confined) while still
wanting an explicit confirm on docker / shell / database writes.

## Why client-agnostic

Some clients can render MCP elicitation prompts; others can't. The
guard's two-call pattern works in every client because the "confirm
token" challenge is just a tool response — the model sees the
description, picks the next call, and the operator's own per-call
approval UI (if any) sits on the executing call. No special client
support required.

## See also

- [`docs/golden-rules.md`](../golden-rules.md) — rule 8 in full,
  including the three permitted patterns (gated off, MCP elicitation,
  guard challenge) and which one each destructive tool uses.
- [`docs/security.md`](../security.md#destructive-action-guards-golden-rule-8)
  — audit-oriented reference, with the per-tool routing list and the
  privilege-boundary discussion.
- [`docs/skills/filesystem.md`](filesystem.md), [`shell.md`](shell.md),
  [`databases.md`](databases.md), [`docker.md`](docker.md),
  [`kubernetes.md`](kubernetes.md), [`git.md`](git.md),
  [`memory.md`](memory.md) — the families this module guards.
