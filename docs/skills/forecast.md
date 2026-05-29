# Forecasting — `forecast_holt_linear`, `forecast_holt_winters`

|  |  |
| --- | --- |
| **Module** | [`src/skills/forecast.rs`](../../src/skills/forecast.rs) |
| **Tools** | `forecast_holt_linear`, `forecast_holt_winters` |
| **Network** | none (pure local compute) |
| **Default** | on; gateable via `[tools]` |

## What it does
Forecasts a numeric time series with **exponential smoothing**. Each method is its
own tool — no hidden auto-selection: you pick the model that matches your data.
Both return per-step point forecasts plus an approximate ~95% prediction interval
(`point ± 1.96·σ·√h`, where σ is the in-sample residual spread).

Smoothing constants can be **pinned explicitly** or, if omitted, are chosen by a
small grid search that minimizes in-sample one-step error. The fitted constants
(α/β/γ) are shown in the output.

Prophet and SARIMAX are Python/statsmodels-heavy and don't fit the single-binary
model, so this is a **deliberate, documented approximation** — solid for short and
medium business-style series (sales, traffic, metrics), not a replacement for a full
statistical model on complex data.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `forecast_holt_linear` | `values`, `horizon`, `alpha?`, `beta?` | Level + **trend** (no seasonality). |
| `forecast_holt_winters` | `values`, `horizon`, `season_length`, `alpha?`, `beta?`, `gamma?` | Level + trend + **additive season**. |

Shared:
- `values` — the series in time order, oldest first (≥2 points; all finite).
- `horizon` — how many future steps to predict (1–500).
- `alpha`/`beta`/`gamma` — smoothing constants in 0–1. Omit any to grid-search it.

`forecast_holt_winters` only:
- `season_length` — cycle length in samples (e.g. **12** for monthly data with a
  yearly cycle, **7** for daily data with a weekly cycle). Must be ≥2, and you need
  at least **2 full seasons** of data (`values.len() ≥ 2 × season_length`).

## Which to use
- **Trend, no repeating cycle** → `forecast_holt_linear`.
- **A repeating seasonal pattern** → `forecast_holt_winters` with the cycle length.

## Example uses
- **Monthly sales, project a year** — `forecast_holt_winters { values: [...24
  months...], horizon: 12, season_length: 12 }`.
- **Daily metric with a weekly rhythm** — `forecast_holt_winters { ..., season_length: 7 }`.
- **Steady trend** — `forecast_holt_linear { values: [...], horizon: 6 }`.
- **Pin the responsiveness** — `forecast_holt_linear { ..., alpha: 0.3, beta: 0.1 }`
  to skip the grid search and control smoothing directly.

## See also
[tools.md](../tools.md)
