//! Yahoo Finance skills (keyless): a richer market-data source than the Stooq
//! `stock_quote`, covering live-ish quotes, OHLC history, and symbol search — the
//! operations a Python user would reach `yfinance` for, without an API key.
//!
//! These hit Yahoo's public JSON endpoints (the same ones `yfinance` uses):
//! `v8/finance/chart/{symbol}` (quote metadata + an OHLC time series) and
//! `v1/finance/search` (symbol/ticker lookup). No key, no crumb/cookie — we
//! deliberately avoid the `quoteSummary` fundamentals endpoint, which now requires
//! a session crumb. Delayed reference data, not a live trading feed. All results
//! are cached. Gated by `[stocks]` (on by default).

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// Format a float compactly: up to 4 decimals, trailing zeros trimmed (so equities
/// read `312.51` while FX reads `1.0843`).
fn num(x: f64) -> String {
    let s = format!("{x:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Epoch seconds → a date (daily+ bars) or date-time (intraday) string in UTC.
fn stamp(ts: i64, intraday: bool) -> String {
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(dt) if intraday => dt.format("%Y-%m-%d %H:%M").to_string(),
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => ts.to_string(),
    }
}

/// True for sub-daily intervals (so history rows show the time component).
fn is_intraday(interval: &str) -> bool {
    interval.ends_with('m') || interval.ends_with('h')
}

/// Fetch and JSON-parse a Yahoo chart response for `symbol`, surfacing Yahoo's own
/// error description (e.g. unknown symbol, bad range) as an invalid-argument error.
async fn fetch_chart(http: &Client, symbol: &str, range: &str, interval: &str) -> Result<Value> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?range={}&interval={}",
        urlencoding(symbol),
        urlencoding(range),
        urlencoding(interval),
    );
    let body = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(serde_json::from_str(&body)?)
}

/// Minimal percent-encoding for path/query segments (symbols can contain `^`, `=`).
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pull Yahoo's `chart.error.description` if the response carries one.
fn chart_error(v: &Value) -> Option<String> {
    let err = v.get("chart")?.get("error")?;
    if err.is_null() {
        return None;
    }
    Some(
        err.get("description")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string(),
    )
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QuoteArgs {
    /// Symbol as Yahoo knows it: `AAPL`, `MSFT`, an index like `^GSPC` (S&P 500),
    /// FX like `EURUSD=X`, crypto like `BTC-USD`. Use `yahoo_search` to find one.
    symbol: String,
}

pub struct YahooQuote;
impl Skill for YahooQuote {
    fn name(&self) -> &'static str {
        "yahoo_quote"
    }
    fn description(&self) -> &'static str {
        "Look up a delayed quote from Yahoo Finance (keyless) for a stock, index, ETF, FX pair, or \
        crypto: current price, day open/high/low, previous close, change + % change, 52-week range, \
        volume, currency, and exchange. Use yahoo_search to resolve a ticker. Delayed reference \
        data, not a live trading feed."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<QuoteArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<QuoteArgs>()?;
            let symbol = args.symbol.trim();
            if symbol.is_empty() {
                return Err(invalid("empty symbol"));
            }
            let key = format!("yahoo_quote|{symbol}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let v = fetch_chart(&server.http, symbol, "1d", "1d")
                .await
                .map_err(internal)?;
            if let Some(e) = chart_error(&v) {
                return Err(invalid(format!("Yahoo: {e}")));
            }
            let meta = v
                .pointer("/chart/result/0/meta")
                .ok_or_else(|| invalid(format!("no quote for '{symbol}' (unknown symbol?)")))?;

            let g = |k: &str| meta.get(k).and_then(Value::as_f64);
            let name = meta
                .get("longName")
                .or_else(|| meta.get("shortName"))
                .and_then(Value::as_str)
                .unwrap_or(symbol);
            let sym = meta.get("symbol").and_then(Value::as_str).unwrap_or(symbol);
            let currency = meta.get("currency").and_then(Value::as_str).unwrap_or("");
            let exch = meta
                .get("fullExchangeName")
                .and_then(Value::as_str)
                .unwrap_or("");

            let header = format!("{name} ({sym})  {exch}");
            let mut lines = vec![header.trim().to_string()];
            if let Some(price) = g("regularMarketPrice") {
                let mut l = format!("  price {} {}", num(price), currency);
                if let Some(prev) = g("chartPreviousClose").or_else(|| g("previousClose")) {
                    let chg = price - prev;
                    let pct = if prev != 0.0 { chg / prev * 100.0 } else { 0.0 };
                    let sign = if chg >= 0.0 { "+" } else { "" };
                    l.push_str(&format!("  ({sign}{} / {sign}{:.2}%)", num(chg), pct));
                }
                lines.push(l.trim_end().to_string());
            }
            if let (Some(lo), Some(hi)) = (g("regularMarketDayLow"), g("regularMarketDayHigh")) {
                lines.push(format!("  day {} – {}", num(lo), num(hi)));
            }
            if let (Some(lo), Some(hi)) = (g("fiftyTwoWeekLow"), g("fiftyTwoWeekHigh")) {
                lines.push(format!("  52wk {} – {}", num(lo), num(hi)));
            }
            if let Some(vol) = g("regularMarketVolume") {
                lines.push(format!("  volume {vol:.0}"));
            }
            if let Some(ts) = meta.get("regularMarketTime").and_then(Value::as_i64) {
                lines.push(format!("  as of {} UTC", stamp(ts, true)));
            }
            let report = lines.join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HistoryArgs {
    /// Symbol as Yahoo knows it (e.g. `AAPL`, `^GSPC`, `BTC-USD`).
    symbol: String,
    /// Look-back window: 1d, 5d, 1mo, 3mo, 6mo, 1y, 2y, 5y, 10y, ytd, max. Default 1mo.
    #[serde(default)]
    range: Option<String>,
    /// Bar size: 1m, 2m, 5m, 15m, 30m, 60m, 90m, 1h, 1d, 5d, 1wk, 1mo, 3mo. Default 1d.
    /// (Intraday intervals only cover recent ranges, per Yahoo's limits.)
    #[serde(default)]
    interval: Option<String>,
}

/// Cap on history rows returned (keeps tool output bounded); the most recent are kept.
const MAX_ROWS: usize = 120;

pub struct YahooHistory;
impl Skill for YahooHistory {
    fn name(&self) -> &'static str {
        "yahoo_history"
    }
    fn description(&self) -> &'static str {
        "Fetch an OHLC price history (date, open/high/low/close, volume) for a symbol from Yahoo \
        Finance (keyless). Choose a range (1d…max, ytd) and bar interval (1m…3mo); intraday \
        intervals only cover recent ranges. Returns up to the 120 most-recent bars. Delayed \
        reference data."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HistoryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<HistoryArgs>()?;
            let symbol = args.symbol.trim();
            if symbol.is_empty() {
                return Err(invalid("empty symbol"));
            }
            let range = args.range.as_deref().unwrap_or("1mo").trim().to_string();
            let interval = args.interval.as_deref().unwrap_or("1d").trim().to_string();
            let key = format!("yahoo_history|{symbol}|{range}|{interval}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let v = fetch_chart(&server.http, symbol, &range, &interval)
                .await
                .map_err(internal)?;
            if let Some(e) = chart_error(&v) {
                return Err(invalid(format!("Yahoo: {e}")));
            }
            let result = v
                .pointer("/chart/result/0")
                .ok_or_else(|| invalid(format!("no history for '{symbol}'")))?;
            let ts = result
                .pointer("/timestamp")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    invalid(format!(
                        "no data points for '{symbol}' at {interval}/{range}"
                    ))
                })?;
            let q = result
                .pointer("/indicators/quote/0")
                .ok_or_else(|| invalid("malformed Yahoo response (no quote series)"))?;
            let col = |k: &str| {
                q.get(k)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            };
            let (open, high, low, close, volume) = (
                col("open"),
                col("high"),
                col("low"),
                col("close"),
                col("volume"),
            );
            let sym = result
                .pointer("/meta/symbol")
                .and_then(Value::as_str)
                .unwrap_or(symbol);
            let currency = result
                .pointer("/meta/currency")
                .and_then(Value::as_str)
                .unwrap_or("");
            let intraday = is_intraday(&interval);

            let n = ts.len();
            let start = n.saturating_sub(MAX_ROWS);
            let mut lines = vec![format!(
                "{sym} — {interval} bars over {range} ({currency})  [date  open  high  low  close  volume]",
            )];
            let cell = |arr: &[Value], i: usize| arr.get(i).and_then(Value::as_f64);
            for i in start..n {
                let Some(t) = ts.get(i).and_then(Value::as_i64) else {
                    continue;
                };
                // Skip rows Yahoo padded with nulls (market holidays / gaps).
                let (Some(o), Some(h), Some(l), Some(c)) = (
                    cell(&open, i),
                    cell(&high, i),
                    cell(&low, i),
                    cell(&close, i),
                ) else {
                    continue;
                };
                let vol = cell(&volume, i).unwrap_or(0.0);
                lines.push(format!(
                    "{}  {}  {}  {}  {}  {:.0}",
                    stamp(t, intraday),
                    num(o),
                    num(h),
                    num(l),
                    num(c),
                    vol
                ));
            }
            if lines.len() == 1 {
                return Err(invalid(format!(
                    "no usable bars for '{symbol}' at {interval}/{range}"
                )));
            }
            if n > MAX_ROWS {
                lines.push(format!("… ({} earlier bars omitted)", n - MAX_ROWS));
            }
            let report = lines.join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    /// Company name or partial ticker, e.g. "apple", "vanguard s&p", "BTC".
    query: String,
}

pub struct YahooSearch;
impl Skill for YahooSearch {
    fn name(&self) -> &'static str {
        "yahoo_search"
    }
    fn description(&self) -> &'static str {
        "Resolve a company/instrument name or partial ticker to Yahoo Finance symbols (keyless). \
        Returns matching symbols with name, type (equity/ETF/index/crypto/FX), and exchange — feed \
        a symbol to yahoo_quote or yahoo_history."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SearchArgs>()?;
            let query = args.query.trim();
            if query.is_empty() {
                return Err(invalid("empty query"));
            }
            let key = format!("yahoo_search|{query}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!(
                "https://query1.finance.yahoo.com/v1/finance/search?q={}&quotesCount=10&newsCount=0",
                urlencoding(query)
            );
            let body = server
                .http
                .get(&url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|e| internal(e.into()))?
                .text()
                .await
                .map_err(|e| internal(e.into()))?;
            let v: Value = serde_json::from_str(&body).map_err(|e| internal(e.into()))?;
            let quotes = v
                .get("quotes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if quotes.is_empty() {
                return Ok(text_result(format!("No symbols found for '{query}'.")));
            }
            let mut lines = vec![format!("Symbols matching '{query}':")];
            for q in quotes.iter() {
                let Some(sym) = q.get("symbol").and_then(Value::as_str) else {
                    continue;
                };
                let name = q
                    .get("longname")
                    .or_else(|| q.get("shortname"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let kind = q
                    .get("typeDisp")
                    .or_else(|| q.get("quoteType"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let exch = q.get("exchDisp").and_then(Value::as_str).unwrap_or("");
                let mut meta: Vec<&str> = Vec::new();
                if !kind.is_empty() {
                    meta.push(kind);
                }
                if !exch.is_empty() {
                    meta.push(exch);
                }
                let tail = if meta.is_empty() {
                    String::new()
                } else {
                    format!("  [{}]", meta.join(", "))
                };
                lines.push(format!("  {sym} — {name}{tail}").trim_end().to_string());
            }
            let report = lines.join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

/// Tool names (gated by `[stocks].enabled`, shared with `stock_quote`).
pub const TOOL_NAMES: &[&str] = &["yahoo_quote", "yahoo_history", "yahoo_search"];

/// The skills this module contributes.
#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// Yahoo Finance's keyless v7 quote endpoint — Yahoo periodically tightens
    /// crumb requirements; this test catches it the day they break.
    #[tokio::test]
    #[ignore]
    async fn yahoo_quote_live() {
        let r = http()
            .get("https://query1.finance.yahoo.com/v7/finance/quote?symbols=AAPL")
            .send()
            .await
            .expect("network");
        // 401/429 = Yahoo tightened things; surface that as a skip so it
        // doesn't block CI but we still see it in stdout.
        if matches!(r.status().as_u16(), 401 | 403 | 429) {
            eprintln!("skipping yahoo_quote_live: status {}", r.status());
            return;
        }
        let r = r.error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let q = &v["quoteResponse"]["result"][0];
        assert_eq!(q["symbol"].as_str(), Some("AAPL"));
    }

    #[tokio::test]
    #[ignore]
    async fn yahoo_search_live() {
        let r = http()
            .get("https://query1.finance.yahoo.com/v1/finance/search?q=apple&quotesCount=3")
            .send()
            .await
            .expect("network");
        if matches!(r.status().as_u16(), 401 | 403 | 429) {
            eprintln!("skipping yahoo_search_live: status {}", r.status());
            return;
        }
        let r = r.error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let quotes = v["quotes"].as_array().expect("missing quotes");
        assert!(!quotes.is_empty());
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(YahooQuote),
        Box::new(YahooHistory),
        Box::new(YahooSearch),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn num_trims_trailing_zeros() {
        assert_eq!(num(312.5100), "312.51");
        assert_eq!(num(1.08430), "1.0843");
        assert_eq!(num(100.0), "100");
        assert_eq!(num(304.989990234375), "304.99");
    }

    #[test]
    fn urlencoding_escapes_symbols() {
        assert_eq!(urlencoding("AAPL"), "AAPL");
        assert_eq!(urlencoding("^GSPC"), "%5EGSPC");
        assert_eq!(urlencoding("EURUSD=X"), "EURUSD%3DX");
        assert_eq!(urlencoding("BTC-USD"), "BTC-USD");
    }

    #[test]
    fn chart_error_extracted() {
        let v: Value = serde_json::from_str(
            r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#,
        )
        .unwrap();
        assert_eq!(
            chart_error(&v).as_deref(),
            Some("No data found, symbol may be delisted")
        );
    }

    #[test]
    fn chart_error_none_on_success() {
        let v: Value =
            serde_json::from_str(r#"{"chart":{"result":[{"meta":{}}],"error":null}}"#).unwrap();
        assert!(chart_error(&v).is_none());
    }

    #[test]
    fn intraday_detection() {
        assert!(is_intraday("5m"));
        assert!(is_intraday("1h"));
        assert!(!is_intraday("1d"));
        assert!(!is_intraday("1wk"));
        assert!(!is_intraday("3mo"));
    }
}
