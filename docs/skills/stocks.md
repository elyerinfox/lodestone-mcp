# Stock quotes — `stock_quote`

|  |  |
| --- | --- |
| **Module** | [`src/skills/stocks.rs`](../../src/skills/stocks.rs) |
| **Tools** | `stock_quote` |
| **Network** | keyless API (Stooq CSV endpoint) |
| **Default** | on — gated by `[stocks]` |
| **Config** | `[stocks]` in [`config/17-data-apis.toml`](../../config/17-data-apis.toml) |

## What it does
Looks up a single delayed stock/index/FX quote by ticker via Stooq's keyless CSV
endpoint — no API key. Neither NYSE nor NASDAQ offers a free public API directly;
Stooq aggregates **delayed** end-of-day/intraday data as plain CSV. This is
reference data, **not a live trading feed**. Results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `stock_quote` | `symbol` | A delayed quote: date/time, open/high/low/close, and volume. |

A bare US-style ticker is normalized to a Stooq symbol (e.g. `AAPL` → `aapl.us`).
Already-suffixed symbols, indices (`^spx` = S&P 500), and 6-char forex (`eurusd`)
are passed through unchanged. Unknown symbols (Stooq returns its `N/D` sentinel)
produce an "unknown symbol?" error.

## Configuration & gating
- `[stocks].enabled` (default `true`, env `LODESTONE_STOCKS_ENABLED`) — exposes the
  `stock_quote` tool. When off, the tool disappears. Also gateable via `[tools]`.
- See [`config/17-data-apis.toml`](../../config/17-data-apis.toml).
- Read-only (no confirmation guard).

## Example uses
- **Equity close** — `stock_quote AAPL` (US assumed) for Apple's latest OHLC + volume.
- **An index** — `stock_quote ^spx` for the S&P 500.
- **A currency pair** — `stock_quote eurusd` for EUR/USD.

## See also
[tools.md](../tools.md)
