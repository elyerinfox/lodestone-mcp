# Databases — `db_list` / `db_query` / `redis_command`

|  |  |
| --- | --- |
| **Module** | [`src/skills/databases.rs`](../../src/skills/databases.rs) |
| **Tools** | `db_list`, `db_query`, `redis_command` |
| **Network** | local-only (database) — connects to your configured instances |
| **Default** | **off** — appears only when a `[databases.<id>]` is configured |
| **Config** | `[databases.<id>]` in [`config/14-databases.toml`](../../config/14-databases.toml) |

## What it does
Queries configured PostgreSQL / MySQL / Redis instances. The family is **off by
default**: its tools appear only when at least one `[databases.<id>]` is configured
(a connection URL is a deliberate, credential-bearing opt-in). Connection URLs are
secrets — they are never returned or logged. **Reads run immediately**; **writes /
DDL** (SQL) and **write / admin commands** (Redis) are destructive and go through the
confirmation guard before they act.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `db_list` | — | read | List the configured databases (id + kind: postgres/mysql/redis); URLs never shown. |
| `db_query` | `database`, `sql`, `confirm?`, `trust?` | read / **destructive** | Run SQL on a postgres/mysql instance. `SELECT`/`WITH`/`SHOW`/`EXPLAIN`/`DESCRIBE`/`VALUES`/`PRAGMA`/`TABLE` read freely; anything else (INSERT/UPDATE/DELETE/DDL) confirms first. Returns rows (capped at 200) or rows-affected. |
| `redis_command` | `database`, `command`, `confirm?`, `trust?` | read / **destructive** | Run a Redis command (parsed like a shell line). Read commands (GET/MGET/HGETALL/KEYS/SCAN/LRANGE/INFO/…) run immediately; writes/admin commands confirm first. |

`database` is a configured `[databases.<id>]`; `db_query` requires kind postgres or
mysql, `redis_command` requires kind redis.

## Configuration & gating
No instances ship by default, so the tools stay hidden until you add at least one
`[databases.<id>]` with `kind` (`postgres` | `mysql` | `redis`) and `url`. The URL is
a credential — keep real URLs in the gitignored `lodestone.toml` (or an
env-substituted config), never commit them.

The first SQL keyword (or Redis command name) classifies the call as read vs. write.
Writes route through the [`guard`](../../src/skills/guard.rs) (golden rule 8): the
first call performs nothing and returns a one-time `confirm` token; call again with
`confirm=<token>` to execute, or `confirm=<token>, trust=true` to stop being asked
for writes to **that database** (keyed per instance) for the rest of the session.
Tokens are single-use and expire after 5 minutes. Set `allow_destructive = true` on
an instance to pre-authorize writes and skip the prompt.

## Example uses
- **Inspect a schema** — `db_list` to see ids, then `db_query` (`database="app", sql="SELECT * FROM users LIMIT 5"`) — a read, runs immediately.
- **Apply a write** — `db_query` (`database="app", sql="UPDATE users SET active=true WHERE id=1"`) returns a token; call again with `confirm=<token>` to run it.
- **Read a Redis key** — `redis_command` (`database="cache", command="HGETALL user:1"`) — a read, runs immediately.
- **Redis write with confirmation** — `redis_command` (`database="cache", command="DEL stale:key"`) returns a token; re-call with `confirm=<token>` to delete.

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
