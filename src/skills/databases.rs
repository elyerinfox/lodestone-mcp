//! Database client skills — query configured PostgreSQL / MySQL / Redis instances.
//!
//! Connections come from `[databases.<id>]` (kind + URL). The family is **off by
//! default**: its tools appear only when at least one instance is configured (a URL
//! is a deliberate, credential-bearing opt-in). Read queries run freely; **writes /
//! DDL** (SQL) and **write / admin commands** (Redis) are routed through the
//! confirmation [`guard`](crate::skills::guard) (golden rule 8), and a per-instance
//! `allow_destructive` pre-authorizes them. URLs are secrets — never returned/logged.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use sqlx::{Column, Row};

use crate::config::DatabaseInstance;
use crate::skills::guard::Decision;
use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result, Lodestone};

pub const TOOL_NAMES: &[&str] = &["db_list", "db_query", "redis_command"];

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

fn instance<'a>(server: &'a Lodestone, id: &str) -> Result<&'a DatabaseInstance, McpError> {
    server.databases.get(id).ok_or_else(|| {
        let known: Vec<&str> = server.databases.keys().map(|s| s.as_str()).collect();
        invalid(format!(
            "no database '{id}' configured (known: {}). Add it under [databases.{id}].",
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        ))
    })
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
    /// Configured database id (a `[databases.<id>]`, kind postgres or mysql).
    database: String,
    /// SQL to run. SELECT/SHOW/EXPLAIN/… read freely; anything else (INSERT/UPDATE/
    /// DELETE/DDL) is destructive and needs confirmation.
    sql: String,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for writes to this database this session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RedisCmdArgs {
    /// Configured database id (a `[databases.<id>]`, kind redis).
    database: String,
    /// Redis command, e.g. `GET mykey` or `HGETALL user:1` (parsed like a shell line).
    command: String,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for writes to this database this session.
    #[serde(default)]
    trust: Option<bool>,
}

// --- skills -----------------------------------------------------------------

pub struct DbList;
impl Skill for DbList {
    fn name(&self) -> &'static str {
        "db_list"
    }
    fn description(&self) -> &'static str {
        "List the configured databases (id and kind: postgres/mysql/redis). Connection URLs are \
        never shown."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            if server.databases.is_empty() {
                return Ok(text_result("No databases configured ([databases.<id>])."));
            }
            let mut ids: Vec<&String> = server.databases.keys().collect();
            ids.sort();
            let mut out = format!("Databases ({}):\n", ids.len());
            for id in ids {
                out.push_str(&format!("  {id}  ({})\n", server.databases[id].kind));
            }
            Ok(text_result(out))
        })
    }
}

pub struct DbQuery;
impl Skill for DbQuery {
    fn name(&self) -> &'static str {
        "db_query"
    }
    fn description(&self) -> &'static str {
        "Run SQL against a configured PostgreSQL or MySQL database. Reads (SELECT/SHOW/EXPLAIN/…) \
        run immediately; writes/DDL are destructive — the first call returns a confirmation token \
        and does nothing, so call again with confirm=<token> (or confirm + trust=true). Returns \
        result rows or rows-affected."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DbQueryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DbQueryArgs>()?;
            let inst = instance(server, &args.database)?;
            let kind = inst.kind.trim().to_ascii_lowercase();
            if kind != "postgres" && kind != "mysql" {
                return Err(invalid(format!(
                    "db_query supports postgres/mysql; '{}' is kind '{}'. Use redis_command for Redis.",
                    args.database, inst.kind
                )));
            }
            let keyword = first_keyword(&args.sql);
            let read = SQL_READ.contains(&keyword.as_str());
            if !read {
                let preview: String = args.sql.trim().chars().take(80).collect();
                let summary = format!("run on {}: {preview}", args.database);
                if let Decision::Challenge(msg) = server.guard.check(
                    &format!("db_query:{}", args.database),
                    "db_query",
                    inst.allow_destructive,
                    &summary,
                    args.confirm.as_deref(),
                    args.trust.unwrap_or(false),
                ) {
                    return Ok(text_result(msg));
                }
            }
            let out = match kind.as_str() {
                "postgres" => run_pg(&inst.url, &args.sql, read).await,
                _ => run_mysql(&inst.url, &args.sql, read).await,
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
        "Run a command against a configured Redis database. Read commands (GET/HGETALL/KEYS/…) run \
        immediately; writes/admin commands are destructive — the first call returns a confirmation \
        token and does nothing, so call again with confirm=<token> (or confirm + trust=true)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RedisCmdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<RedisCmdArgs>()?;
            let inst = instance(server, &args.database)?;
            if !inst.kind.trim().eq_ignore_ascii_case("redis") {
                return Err(invalid(format!(
                    "redis_command needs a redis database; '{}' is kind '{}'.",
                    args.database, inst.kind
                )));
            }
            let parts = shell_words::split(args.command.trim())
                .map_err(|e| invalid(format!("could not parse command: {e}")))?;
            if parts.is_empty() {
                return Err(invalid("empty command"));
            }
            let name = parts[0].to_ascii_uppercase();
            let read = REDIS_READ.contains(&name.as_str());
            if !read {
                let summary = format!("{} on {}", args.command.trim(), args.database);
                if let Decision::Challenge(msg) = server.guard.check(
                    &format!("redis_command:{}", args.database),
                    "redis_command",
                    inst.allow_destructive,
                    &summary,
                    args.confirm.as_deref(),
                    args.trust.unwrap_or(false),
                ) {
                    return Ok(text_result(msg));
                }
            }
            let out = run_redis(&inst.url, &parts).await.map_err(internal)?;
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
    vec![Box::new(DbList), Box::new(DbQuery), Box::new(RedisCommand)]
}

#[cfg(test)]
mod tests {
    use super::{first_keyword, format_redis};

    #[test]
    fn classifies_sql_keyword() {
        assert_eq!(first_keyword("  select * from t"), "SELECT");
        assert_eq!(first_keyword("WITH x AS (...) SELECT"), "WITH");
        assert_eq!(first_keyword("DELETE FROM t"), "DELETE");
        assert_eq!(first_keyword("(SELECT 1)"), "SELECT");
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
