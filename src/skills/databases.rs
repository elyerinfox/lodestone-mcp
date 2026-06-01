//! Database client skills — query PostgreSQL / MySQL / Redis.
//!
//! **No preconfiguration:** there are no stored connections. The caller passes a
//! `connection` URL in each call — the credentials the user hands the model in
//! conversation — and connectivity happens through that exchange. The family is
//! gated by `[databases].enabled` (off by default). Read queries run freely; **writes
//! / DDL** (SQL) and **write / admin commands** (Redis) are routed through the
//! confirmation [`guard`](crate::skills::guard) (golden rule 8); `[databases].
//! allow_destructive` pre-authorizes. Connection URLs are secrets — they are never
//! returned or logged (summaries/errors show only scheme + host).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use sqlx::{Column, Row};

use crate::skills::guard::Decision;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result};

/// Max rows rendered from a query result (the query still runs in full).
const MAX_ROWS: usize = 200;

/// SQL statements that only read — anything else is treated as destructive.
const SQL_READ: &[&str] = &[
    "SELECT", "WITH", "SHOW", "EXPLAIN", "DESCRIBE", "DESC", "VALUES", "PRAGMA", "TABLE",
];

/// Redis commands that only read — anything else is treated as a write/admin op.
const REDIS_READ: &[&str] = &[
    "GET",
    "MGET",
    "STRLEN",
    "EXISTS",
    "TYPE",
    "TTL",
    "PTTL",
    "KEYS",
    "SCAN",
    "HGET",
    "HMGET",
    "HGETALL",
    "HKEYS",
    "HVALS",
    "HLEN",
    "HEXISTS",
    "LRANGE",
    "LLEN",
    "LINDEX",
    "SMEMBERS",
    "SCARD",
    "SISMEMBER",
    "ZRANGE",
    "ZREVRANGE",
    "ZCARD",
    "ZSCORE",
    "ZRANK",
    "GETRANGE",
    "DBSIZE",
    "RANDOMKEY",
    "PING",
    "INFO",
    "TIME",
    "GETBIT",
    "BITCOUNT",
    "LPOS",
    "MEMORY",
    "OBJECT",
    "DUMP",
];

/// Engine implied by a connection URL's scheme, or `None` if unrecognized.
fn scheme_kind(url: &str) -> Option<&'static str> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
        Some("postgres")
    } else if lower.starts_with("mysql://") {
        Some("mysql")
    } else if lower.starts_with("redis://") || lower.starts_with("rediss://") {
        Some("redis")
    } else {
        None
    }
}

/// A credential-free label for a connection URL (drops any `user:pass@`), for use in
/// confirmation summaries and errors. Never expose the raw URL.
fn redact(url: &str) -> String {
    match url.trim().split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
            format!("{scheme}://{host}")
        }
        None => "(connection)".to_string(),
    }
}

/// First SQL keyword (uppercased), used to classify read vs. write.
fn first_keyword(sql: &str) -> String {
    sql.trim_start()
        .split(|c: char| c.is_whitespace() || c == '(' || c == ';')
        .find(|t| !t.is_empty())
        .unwrap_or("")
        .to_ascii_uppercase()
}

// --- argument schemas -------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DbQueryArgs {
    /// Connection URL: `postgres://user:pass@host:5432/db` or `mysql://…`. Provided in
    /// the call (no preconfiguration); the engine is inferred from the scheme.
    connection: String,
    /// SQL to run. SELECT/SHOW/EXPLAIN/… read freely; anything else (INSERT/UPDATE/
    /// DELETE/DDL) is destructive and needs confirmation.
    sql: String,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for writes to this connection this session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RedisCmdArgs {
    /// Connection URL: `redis://host:6379` or `rediss://…`. Provided in the call.
    connection: String,
    /// Redis command, e.g. `GET mykey` or `HGETALL user:1` (parsed like a shell line).
    command: String,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for writes to this connection this session.
    #[serde(default)]
    trust: Option<bool>,
}

// --- skills -----------------------------------------------------------------

pub struct DbQuery;
impl Skill for DbQuery {
    fn name(&self) -> &'static str {
        "db_query"
    }
    fn description(&self) -> &'static str {
        "Run SQL against PostgreSQL or MySQL using a `connection` URL you pass in (e.g. \
        postgres://user:pass@host/db) — no preconfiguration. Reads (SELECT/SHOW/EXPLAIN/…) run \
        immediately; writes/DDL are destructive — the first call returns a confirmation token and \
        does nothing, so call again with confirm=<token> (or confirm + trust=true). Returns result \
        rows or rows-affected."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DbQueryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DbQueryArgs>()?;
            let conn = args.connection.trim();
            let kind = scheme_kind(conn).ok_or_else(|| {
                invalid("connection must be a postgres:// (or postgresql://) or mysql:// URL")
            })?;
            if kind == "redis" {
                return Err(invalid("that's a redis:// URL — use redis_command instead"));
            }
            let keyword = first_keyword(&args.sql);
            let read = SQL_READ.contains(&keyword.as_str());
            if !read {
                let preview: String = args.sql.trim().chars().take(80).collect();
                let summary = format!("run on {}: {preview}", redact(conn));
                if let Decision::Challenge(msg) = server.guard.check(
                    &format!("db_query:{conn}"),
                    "db_query",
                    server.databases.allow_destructive,
                    &summary,
                    args.confirm.as_deref(),
                    args.trust.unwrap_or(false),
                ) {
                    return Ok(text_result(msg));
                }
            }
            let out = match kind {
                "postgres" => run_pg(conn, &args.sql, read).await,
                _ => run_mysql(conn, &args.sql, read).await,
            }
            .map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

pub struct RedisCommand;
impl Skill for RedisCommand {
    fn name(&self) -> &'static str {
        "redis_command"
    }
    fn description(&self) -> &'static str {
        "Run a Redis command using a `connection` URL you pass in (redis://host:6379) — no \
        preconfiguration. Read commands (GET/HGETALL/KEYS/…) run immediately; writes/admin commands \
        are destructive — the first call returns a confirmation token and does nothing, so call \
        again with confirm=<token> (or confirm + trust=true)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RedisCmdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<RedisCmdArgs>()?;
            let conn = args.connection.trim();
            if scheme_kind(conn) != Some("redis") {
                return Err(invalid("connection must be a redis:// or rediss:// URL"));
            }
            let parts = shell_words::split(args.command.trim())
                .map_err(|e| invalid(format!("could not parse command: {e}")))?;
            if parts.is_empty() {
                return Err(invalid("empty command"));
            }
            let name = parts[0].to_ascii_uppercase();
            let read = REDIS_READ.contains(&name.as_str());
            if !read {
                let summary = format!("{} on {}", args.command.trim(), redact(conn));
                if let Decision::Challenge(msg) = server.guard.check(
                    &format!("redis_command:{conn}"),
                    "redis_command",
                    server.databases.allow_destructive,
                    &summary,
                    args.confirm.as_deref(),
                    args.trust.unwrap_or(false),
                ) {
                    return Ok(text_result(msg));
                }
            }
            let out = run_redis(conn, &parts).await.map_err(internal)?;
            Ok(text_result(truncate_chars(&out, server.max_chars)))
        })
    }
}

// --- engines ----------------------------------------------------------------

async fn run_pg(url: &str, sql: &str, read: bool) -> Result<String> {
    use sqlx::postgres::PgPoolOptions;
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .context("connecting to PostgreSQL")?;
    let result = if read {
        sqlx::query(sql).fetch_all(&pool).await.map(|rows| {
            render_table(
                rows.len(),
                || {
                    rows.iter()
                        .take(MAX_ROWS)
                        .map(|r| {
                            let cols = r.columns().len();
                            (0..cols).map(|i| pg_cell(r, i)).collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                },
                rows.first().map(header),
            )
        })
    } else {
        sqlx::query(sql)
            .execute(&pool)
            .await
            .map(|r| format!("OK ({} row(s) affected)", r.rows_affected()))
    };
    pool.close().await;
    result.context("running query")
}

async fn run_mysql(url: &str, sql: &str, read: bool) -> Result<String> {
    use sqlx::mysql::MySqlPoolOptions;
    let pool = MySqlPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await
        .context("connecting to MySQL")?;
    let result = if read {
        sqlx::query(sql).fetch_all(&pool).await.map(|rows| {
            render_table(
                rows.len(),
                || {
                    rows.iter()
                        .take(MAX_ROWS)
                        .map(|r| {
                            let cols = r.columns().len();
                            (0..cols).map(|i| mysql_cell(r, i)).collect::<Vec<_>>()
                        })
                        .collect::<Vec<_>>()
                },
                rows.first().map(header),
            )
        })
    } else {
        sqlx::query(sql)
            .execute(&pool)
            .await
            .map(|r| format!("OK ({} row(s) affected)", r.rows_affected()))
    };
    pool.close().await;
    result.context("running query")
}

/// Column names of a row, for the table header.
fn header<R: Row>(row: &R) -> Vec<String> {
    row.columns().iter().map(|c| c.name().to_string()).collect()
}

/// Assemble a `header | … \n rows…` table from already-stringified cells.
fn render_table(
    total: usize,
    cells: impl FnOnce() -> Vec<Vec<String>>,
    header: Option<Vec<String>>,
) -> String {
    if total == 0 {
        return "(0 rows)".to_string();
    }
    let mut out = format!("{total} row(s):\n");
    if let Some(h) = header {
        out.push_str(&h.join(" | "));
        out.push('\n');
    }
    for row in cells() {
        out.push_str(&row.join(" | "));
        out.push('\n');
    }
    if total > MAX_ROWS {
        out.push_str(&format!("… ({} more row(s) not shown)\n", total - MAX_ROWS));
    }
    out
}

/// Render one PostgreSQL cell, probing common types and falling back safely so an
/// unsupported column type never fails the whole query.
fn pg_cell(row: &sqlx::postgres::PgRow, i: usize) -> String {
    use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
    if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<i32>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<DateTime<Utc>>, _>(i) {
        return opt(v.map(|x| x.to_rfc3339()));
    }
    if let Ok(v) = row.try_get::<Option<NaiveDateTime>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<NaiveDate>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return opt(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(i) {
        return opt(v.map(|b| format!("<{} bytes>", b.len())));
    }
    "<?>".to_string()
}

/// Render one MySQL cell (same probing strategy as `pg_cell`).
fn mysql_cell(row: &sqlx::mysql::MySqlRow, i: usize) -> String {
    use chrono::{NaiveDate, NaiveDateTime};
    if let Ok(v) = row.try_get::<Option<bool>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<i64>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<f64>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<NaiveDateTime>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<NaiveDate>, _>(i) {
        return opt(v.map(|x| x.to_string()));
    }
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return opt(v);
    }
    if let Ok(v) = row.try_get::<Option<Vec<u8>>, _>(i) {
        return opt(v.map(|b| format!("<{} bytes>", b.len())));
    }
    "<?>".to_string()
}

fn opt(v: Option<String>) -> String {
    v.unwrap_or_else(|| "NULL".to_string())
}

async fn run_redis(url: &str, parts: &[String]) -> Result<String> {
    let client = redis::Client::open(url).context("opening Redis client")?;
    let mut conn = client
        .get_connection_manager()
        .await
        .context("connecting to Redis")?;
    let mut cmd = redis::cmd(&parts[0]);
    for p in &parts[1..] {
        cmd.arg(p);
    }
    let value: redis::Value = cmd
        .query_async(&mut conn)
        .await
        .context("running Redis command")?;
    Ok(format_redis(&value))
}

/// Render a Redis reply value as readable text.
fn format_redis(v: &redis::Value) -> String {
    match v {
        redis::Value::Nil => "(nil)".to_string(),
        redis::Value::Int(i) => i.to_string(),
        redis::Value::Double(d) => d.to_string(),
        redis::Value::Boolean(b) => b.to_string(),
        redis::Value::SimpleString(s) => s.clone(),
        redis::Value::Okay => "OK".to_string(),
        redis::Value::BulkString(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        redis::Value::Array(items) | redis::Value::Set(items) => items
            .iter()
            .map(format_redis)
            .collect::<Vec<_>>()
            .join("\n"),
        redis::Value::Map(pairs) => pairs
            .iter()
            .map(|(k, val)| format!("{}: {}", format_redis(k), format_redis(val)))
            .collect::<Vec<_>>()
            .join("\n"),
        other => format!("{other:?}"),
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(DbQuery), Box::new(RedisCommand)]
}

#[cfg(test)]
mod tests {
    use super::{first_keyword, format_redis, redact, scheme_kind};

    #[test]
    fn classifies_sql_keyword() {
        assert_eq!(first_keyword("  select * from t"), "SELECT");
        assert_eq!(first_keyword("WITH x AS (...) SELECT"), "WITH");
        assert_eq!(first_keyword("DELETE FROM t"), "DELETE");
        assert_eq!(first_keyword("(SELECT 1)"), "SELECT");
    }

    #[test]
    fn infers_engine_from_scheme() {
        assert_eq!(scheme_kind("postgres://u@h/db"), Some("postgres"));
        assert_eq!(scheme_kind("postgresql://h/db"), Some("postgres"));
        assert_eq!(scheme_kind("mysql://h/db"), Some("mysql"));
        assert_eq!(scheme_kind("redis://h:6379"), Some("redis"));
        assert_eq!(scheme_kind("rediss://h"), Some("redis"));
        assert_eq!(scheme_kind("http://h"), None);
    }

    #[test]
    fn redact_hides_credentials() {
        assert_eq!(
            redact("postgres://user:secret@db.example:5432/app"),
            "postgres://db.example:5432/app"
        );
        assert_eq!(redact("redis://cache:6379"), "redis://cache:6379");
        assert!(!redact("mysql://u:p@h/d").contains("p@"));
    }

    #[test]
    fn formats_redis_values() {
        use redis::Value;
        assert_eq!(format_redis(&Value::Nil), "(nil)");
        assert_eq!(format_redis(&Value::Int(7)), "7");
        assert_eq!(
            format_redis(&Value::BulkString(b"hi".to_vec())),
            "hi".to_string()
        );
        let arr = Value::Array(vec![Value::Int(1), Value::BulkString(b"a".to_vec())]);
        assert_eq!(format_redis(&arr), "1\na");
    }
}
