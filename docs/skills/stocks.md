# Stock & market quotes — `stock_quote`, `yahoo_*`

|  |  |
| --- | --- |
| **Modules** | [`src/skills/stocks.rs`](../../src/skills/stocks.rs) (Stooq), [`src/skills/yahoo.rs`](../../src/skills/yahoo.rs) (Yahoo Finance) |
| **Tools** | `stock_quote`, `yahoo_quote`, `yahoo_history`, `yahoo_search` |
| **Network** | keyless (Stooq CSV; Yahoo Finance public JSON) |
| **Default** | on — gated by `[stocks]` |
| **Config** | `[stocks]` in [`config/17-data-apis.toml`](../../config/17-data-apis.toml) |

## What it does
Keyless market data from two complementary sources. Both are **delayed reference
data, not a live trading feed**, and all results are cached.

- **Stooq** (`stock_quote`) — one quick OHLC + volume line per symbol, via a plain
  CSV endpoint. Neither NYSE nor NASDAQ offers a free public API directly; Stooq
  aggregates delayed end-of-day/intraday data.
- **Yahoo Finance** (`yahoo_*`) — the richer source a Python user reaches `yfinance`
  for: a fuller quote (change/%, day & 52-week range, exchange, currency), an OHLC
  **history** over a chosen range/interval, and **symbol search**. Uses Yahoo's
  public JSON endpoints (`v8/finance/chart`, `v1/finance/search`) — no key, and no
  session crumb/cookie (the crumb-gated `quoteSummary` fundamentals endpoint is
  deliberately avoided).

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `stock_quote` | `symbol` | A delayed Stooq quote: date/time, open/high/low/close, and volume. |
| `yahoo_quote` | `symbol` | A delayed Yahoo quote: price, change + % change, day & 52-week range, volume, currency, exchange. |
| `yahoo_history` | `symbol`, `range?`, `interval?` | OHLC history (date, O/H/L/C, volume); up to the 120 most-recent bars. |
| `yahoo_search` | `query` | Resolve a name / partial ticker to Yahoo symbols (type + exchange). |

**Symbols.** For `stock_quote`, a bare US ticker is normalized to a Stooq symbol
(`AAPL` → `aapl.us`); indices (`^spx`) and 6-char forex (`eurusd`) pass through.
For the `yahoo_*` tools, use Yahoo's own symbology — `AAPL`, an index like `^GSPC`,
FX like `EURUSD=X`, crypto like `BTC-USD` — and `yahoo_search` to discover one.

**History range/interval.** `range` ∈ {1d, 5d, 1mo (default), 3mo, 6mo, 1y, 2y, 5y,
10y, ytd, max}; `interval` ∈ {1m, 2m, 5m, 15m, 30m, 60m, 90m, 1h, 1d (default), 5d,
1wk, 1mo, 3mo}. Intraday intervals only cover recent ranges (a Yahoo limit); an
invalid pairing surfaces Yahoo's own error message.

## Configuration & gating
- `[stocks].enabled` (default `true`, env `LODESTONE_STOCKS_ENABLED`) — exposes
  **all** of `stock_quote` and the `yahoo_*` tools. When off, they disappear. Each is
  also independently gateable via `[tools]`.
- See [`config/17-data-apis.toml`](../../config/17-data-apis.toml).
- Read-only (no confirmation guard).

## Example uses
- **Quick close** — `stock_quote AAPL` for Apple's latest OHLC + volume.
- **Rich quote** — `yahoo_quote AAPL` for price, change %, ranges, and exchange.
- **Price history** — `yahoo_history { symbol: "MSFT", range: "6mo", interval: "1d" }`
  for half a year of daily bars (e.g. to chart or feed `forecast`).
- **Find a ticker** — `yahoo_search "vanguard s&p"` → `VOO` and friends.
- **Beyond equities** — `yahoo_quote ^GSPC` (S&P 500 index), `yahoo_quote EURUSD=X`
  (FX), `yahoo_quote BTC-USD` (crypto).

## See also
[tools.md](../tools.md)
