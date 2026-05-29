# Databases — `db_query` / `redis_command`

|  |  |
| --- | --- |
| **Module** | [`src/skills/databases.rs`](../../src/skills/databases.rs) |
| **Tools** | `db_query`, `redis_command` |
| **Network** | local-only (database) — connects to whatever URL you pass |
| **Default** | **off** — gated by `[databases].enabled` |
| **Config** | `[databases]` in [`config/14-databases.toml`](../../config/14-databases.toml) |

## What it does
Queries PostgreSQL / MySQL / Redis. **No preconfiguration**: there are no stored
connections — you give the model a connection URL in conversation and it passes that
URL on each call, so connectivity happens through the exchange. The family is **off
by default** (`[databases].enabled`). The engine is inferred from the URL scheme
(`postgres://`/`postgresql://`, `mysql://`, `redis://`/`rediss://`). Connection URLs
are secrets — never returned or logged (summaries/errors show only scheme + host).
**Reads run immediately**; **writes / DDL** (SQL) and **write / admin commands**
(Redis) are destructive and go through the confirmation guard first.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `db_query` | `connection`, `sql`, `confirm?`, `trust?` | read / **destructive** | Run SQL on a `postgres://`/`mysql://` connection. `SELECT`/`WITH`/`SHOW`/`EXPLAIN`/`DESCRIBE`/`VALUES`/`PRAGMA`/`TABLE` read freely; anything else (INSERT/UPDATE/DELETE/DDL) confirms first. Returns rows (capped at 200) or rows-affected. |
| `redis_command` | `connection`, `command`, `confirm?`, `trust?` | read / **destructive** | Run a Redis command (parsed like a shell line) on a `redis://` connection. Read commands (GET/MGET/HGETALL/KEYS/SCAN/LRANGE/INFO/…) run immediately; writes/admin commands confirm first. |

## Configuration & gating
Set `[databases].enabled = true` to expose the tools (off by default). The first SQL
keyword (or Redis command name) classifies the call as read vs. write. Writes route
through the [`guard`](../../src/skills/guard.rs) (golden rule 8): the first call
performs nothing and returns a one-time `confirm` token; call again with
`confirm=<token>` to execute, or `confirm=<token>, trust=true` to stop being asked for
writes to **that connection** (keyed per URL) for the rest of the session. Tokens are
single-use and expire after 5 minutes. `[databases].allow_destructive = true`
pre-authorizes writes and skips the prompt.

## Example uses
- **Read** — `db_query` (`connection="postgres://user:pass@host/app", sql="SELECT * FROM users LIMIT 5"`) — runs immediately.
- **Write (confirmed)** — `db_query` (`connection=…, sql="UPDATE users SET active=true WHERE id=1"`) returns a token; call again with `confirm=<token>` to run it.
- **Redis read** — `redis_command` (`connection="redis://cache:6379", command="HGETALL user:1"`).
- **Redis write (confirmed)** — `redis_command` (`connection=…, command="DEL stale:key"`) returns a token; re-call with `confirm=<token>`.

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
