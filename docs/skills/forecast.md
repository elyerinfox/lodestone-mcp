# Forecasting — `forecast`

|  |  |
| --- | --- |
| **Module** | [`src/skills/forecast.rs`](../../src/skills/forecast.rs) |
| **Tools** | `forecast` |
| **Network** | none (pure local compute) |
| **Default** | on; gateable via `[tools]` |

## What it does
Forecasts a numeric time series with **exponential smoothing**: Holt's linear-trend
method, or Holt-Winters additive (level + trend + season) when you give a season
length. Smoothing constants are chosen by a small grid search that minimizes
in-sample one-step error. Returns per-step point forecasts plus an approximate
~95% prediction interval (`point ± 1.96·σ·√h`, where σ is the residual spread).

Prophet and SARIMAX are Python/statsmodels-heavy and don't fit the single-binary
model, so this is a **deliberate, documented approximation** — solid for short and
medium business-style series (sales, traffic, metrics), not a replacement for a full
statistical model on complex data.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `forecast` | `values`, `horizon`, `season_length?` | Forecast `horizon` future steps from `values` (oldest first). |

- `values` — the series in time order, oldest first (≥2 points).
- `horizon` — how many future steps to predict (1–500).
- `season_length` — optional cycle length in samples (e.g. **12** for monthly data
  with a yearly cycle, **7** for daily data with a weekly cycle). Needs ≥2 full
  periods; omit for a trend-only (non-seasonal) forecast.

## Method selection
- **Holt-Winters additive** when `season_length ≥ 2` and there are ≥2 full periods.
- **Holt linear trend** for any series of ≥3 points without a usable season.
- **Naive** (carry the last value) when the series is too short to fit a trend.

The chosen method and fitted constants (α/β/γ) are shown in the output.

## Example uses
- **Monthly sales, project a year** — `forecast { values: [...24 months...],
  horizon: 12, season_length: 12 }`.
- **Daily metric with a weekly rhythm** — `season_length: 7`.
- **Pair with market data** — feed `yahoo_history` closes in to extrapolate a trend
  (reference only — markets are not smoothing-predictable).

## See also
[tools.md](../tools.md)
