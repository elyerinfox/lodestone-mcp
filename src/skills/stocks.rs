//! Stock-quote skill (keyless): a delayed quote via Stooq's CSV endpoint — no API
//! key. Reference/delayed data, not a live trading feed. Results are cached.
//!
//! Neither NYSE nor NASDAQ offers a free public API directly; Stooq aggregates
//! delayed end-of-day/intraday data for US tickers (and many others) as plain CSV.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QuoteArgs {
    /// Ticker symbol, e.g. `AAPL` (US assumed → `aapl.us`), `MSFT`, or a full Stooq
    /// symbol like `^spx` (S&P 500 index) or `eurusd`.
    symbol: String,
}

/// Normalize a user ticker to a Stooq symbol: lowercase; append `.us` for a bare
/// US-style ticker (no `.` and not an index `^`).
fn stooq_symbol(input: &str) -> String {
    let s = input.trim().to_ascii_lowercase();
    // Append `.us` only for short, bare tickers; leave full Stooq symbols alone
    // (already-suffixed `aapl.us`, indices `^spx`, and 6-char forex like `eurusd`).
    if s.is_empty() || s.contains('.') || s.starts_with('^') || s.len() > 5 {
        s
    } else {
        format!("{s}.us")
    }
}

/// One parsed quote row.
struct Quote {
    symbol: String,
    date: String,
    time: String,
    open: String,
    high: String,
    low: String,
    close: String,
    volume: String,
}

/// Parse Stooq's `f=sd2t2ohlcv` CSV (a header row + one data row). Returns `None`
/// for the "N/D" (no data) sentinel Stooq emits for unknown symbols.
fn parse_csv(body: &str) -> Option<Quote> {
    let mut lines = body.lines().filter(|l| !l.trim().is_empty());
    let _header = lines.next()?;
    let row = lines.next()?;
    let f: Vec<&str> = row.split(',').map(str::trim).collect();
    if f.len() < 8 {
        return None;
    }
    // Stooq returns "N/D" in OHLC when it has no data for the symbol.
    if f[3].eq_ignore_ascii_case("N/D") || f[6].eq_ignore_ascii_case("N/D") {
        return None;
    }
    Some(Quote {
        symbol: f[0].to_string(),
        date: f[1].to_string(),
        time: f[2].to_string(),
        open: f[3].to_string(),
        high: f[4].to_string(),
        low: f[5].to_string(),
        close: f[6].to_string(),
        volume: f[7].to_string(),
    })
}

async fn fetch_csv(http: &Client, symbol: &str) -> Result<String> {
    let url = format!("https://stooq.com/q/l/?s={symbol}&f=sd2t2ohlcv&h&e=csv");
    Ok(http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

pub struct StockQuote;
impl Skill for StockQuote {
    fn name(&self) -> &'static str {
        "stock_quote"
    }
    fn description(&self) -> &'static str {
        "Look up a delayed stock/index/FX quote by ticker via the keyless Stooq CSV endpoint (no API \
        key). US tickers are assumed (e.g. AAPL → aapl.us); pass a full Stooq symbol for others \
        (^spx, eurusd). Returns date/time, open/high/low/close, and volume. Delayed reference data, \
        not a live trading feed."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<QuoteArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<QuoteArgs>()?;
            let symbol = stooq_symbol(&args.symbol);
            if symbol.is_empty() {
                return Err(invalid("empty symbol"));
            }
            let key = format!("stock|{symbol}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let body = fetch_csv(&server.http, &symbol).await.map_err(internal)?;
            let q = parse_csv(&body).ok_or_else(|| {
                invalid(format!("no quote for '{}' (unknown symbol?)", args.symbol))
            })?;
            let out = format!(
                "{} — {} {}\n  open {}  high {}  low {}  close {}\n  volume {}",
                q.symbol, q.date, q.time, q.open, q.high, q.low, q.close, q.volume,
            );
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// Tool name (gated by `[stocks].enabled`).
pub const TOOL_NAMES: &[&str] = &["stock_quote"];

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(StockQuote)]
}

#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// Stooq's CSV quote endpoint — keyless, the source `stock_quote` reads.
    #[tokio::test]
    #[ignore]
    async fn stooq_quote_live() {
        let r = http()
            .get("https://stooq.com/q/l/?s=aapl.us&i=d&f=sd2t2ohlcv&h&e=csv")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let body = r.text().await.unwrap();
        // First line is the CSV header; second line carries the quote.
        let mut lines = body.lines();
        let header = lines.next().expect("no CSV header");
        for col in ["Symbol", "Date", "Open", "High", "Low", "Close", "Volume"] {
            assert!(header.contains(col), "missing column {col}");
        }
        let row = lines.next().expect("no CSV row");
        assert!(
            row.to_uppercase().contains("AAPL"),
            "row missing AAPL: {row}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_csv, stooq_symbol};

    #[test]
    fn normalizes_symbols() {
        assert_eq!(stooq_symbol("AAPL"), "aapl.us");
        assert_eq!(stooq_symbol("aapl.us"), "aapl.us");
        assert_eq!(stooq_symbol("^SPX"), "^spx");
        assert_eq!(stooq_symbol("eurusd"), "eurusd");
    }

    #[test]
    fn parses_quote_csv() {
        let csv = "Symbol,Date,Time,Open,High,Low,Close,Volume\nAAPL.US,2024-05-28,22:00:05,189.5,191.0,189.1,190.29,12345678\n";
        let q = parse_csv(csv).unwrap();
        assert_eq!(q.symbol, "AAPL.US");
        assert_eq!(q.close, "190.29");
        assert_eq!(q.volume, "12345678");
    }

    #[test]
    fn rejects_no_data() {
        let csv =
            "Symbol,Date,Time,Open,High,Low,Close,Volume\nNOPE.US,N/D,N/D,N/D,N/D,N/D,N/D,N/D\n";
        assert!(parse_csv(csv).is_none());
    }
}
