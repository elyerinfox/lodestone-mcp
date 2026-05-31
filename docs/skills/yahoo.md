# Yahoo Finance — `yahoo_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/yahoo.rs`](../../src/skills/yahoo.rs) |
| **Tools** | `yahoo_quote`, `yahoo_history`, `yahoo_search` |
| **Network** | Yahoo Finance public JSON endpoints (`query1.finance.yahoo.com`) — **keyless** |
| **Default** | **on** — gated by `[stocks]` |
| **Config** | `[stocks]` in [`config/16-stocks.toml`](../../config/16-stocks.toml). Shares config with [`stocks`](stocks.md) (Stooq). |

## What it does

A richer market-data source than the Stooq `stock_quote` — covering
**live-ish quotes, OHLC history, and symbol search** — the operations a
Python user would reach `yfinance` for, without an API key.

These hit Yahoo's public JSON endpoints (the same ones `yfinance` uses):
`v8/finance/chart/{symbol}` (quote metadata + an OHLC time series) and
`v1/finance/search` (symbol / ticker lookup). No key, no crumb / cookie — the
project deliberately avoids the `quoteSummary` fundamentals endpoint, which
now requires a session crumb. **Delayed reference data, not a live trading
feed.**

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `yahoo_quote` | `symbol` | Delayed quote: price, change / %, day & 52-week range, volume, currency, exchange. |
| `yahoo_history` | `symbol`, `range?`, `interval?` | OHLC price history (date + O/H/L/C + volume). `range`: `1d` / `5d` / `1mo` / … / `max`; `interval`: `1m` / `5m` / `1h` / `1d` / `1wk` / `1mo`. |
| `yahoo_search` | `query` | Resolve a company name / partial ticker to Yahoo symbols (type + exchange). |

## Example uses

- **Today** — `yahoo_quote { symbol: "AAPL" }`.
- **One-year daily history for `chart_candlestick`** —
  `yahoo_history { symbol: "AAPL", range: "1y", interval: "1d" }`.
- **Find the ticker** — `yahoo_search { query: "Cloudflare" }`.

## Notes

- **Delayed.** Real-time use requires a paid feed; Yahoo's public endpoints
  are 15-20 min delayed for most exchanges.
- **No fundamentals** (`quoteSummary` needs auth). Stick to quotes / history.
- **Cached.** Results pass through the retrieval cache.

## See also

- [tools.md](../tools.md)
- [skills/stocks.md](stocks.md) — Stooq `stock_quote` covers many indices /
  FX symbols Yahoo doesn't.
- [skills/chart.md](chart.md) — `chart_candlestick` consumes
  `yahoo_history`-shaped OHLC data.
