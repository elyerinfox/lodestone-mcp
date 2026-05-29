//! Time-series forecasting skill (local, no network, no dependency): exponential
//! smoothing with trend and optional seasonality (Holt / Holt-Winters additive),
//! hand-rolled in pure Rust.
//!
//! Prophet and SARIMAX are Python/statsmodels-heavy and don't fit the single-binary
//! model, so this is a deliberate, documented approximation: triple exponential
//! smoothing (level + trend + season) with a small grid search over the smoothing
//! constants to minimize in-sample one-step error, plus a rough widening prediction
//! interval from the residual spread. Good for short/medium business-style series;
//! not a substitute for a full statistical model on complex data.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::invalid;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::text_result;

/// Format a float compactly (≤4 decimals, trailing zeros trimmed).
fn num(x: f64) -> String {
    if !x.is_finite() {
        return "n/a".to_string();
    }
    let s = format!("{x:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Outcome of fitting + forecasting: the per-step point forecasts, the in-sample
/// residual std (for intervals), the method label, and the fitted constants.
struct Fit {
    forecasts: Vec<f64>,
    sigma: f64,
    method: String,
}

/// Holt-Winters **additive** smoothing. `m` is the season length. Returns the
/// horizon forecasts and the in-sample one-step residual std (warm-up skipped).
fn holt_winters(
    y: &[f64],
    m: usize,
    alpha: f64,
    beta: f64,
    gamma: f64,
    horizon: usize,
) -> (Vec<f64>, f64) {
    let n = y.len();
    let l0 = y[..m].iter().sum::<f64>() / m as f64;
    let b0 = if n >= 2 * m {
        let m2 = y[m..2 * m].iter().sum::<f64>() / m as f64;
        (m2 - l0) / m as f64
    } else {
        (y[n - 1] - y[0]) / (n as f64 - 1.0)
    };
    let mut season: Vec<f64> = (0..m).map(|i| y[i] - l0).collect();
    let (mut level, mut trend) = (l0, b0);
    let mut sse = 0.0;
    let mut cnt = 0.0;
    for (t, &yt) in y.iter().enumerate() {
        let si = t % m;
        let pred = level + trend + season[si];
        if t >= m {
            sse += (yt - pred).powi(2);
            cnt += 1.0;
        }
        let new_level = alpha * (yt - season[si]) + (1.0 - alpha) * (level + trend);
        let new_trend = beta * (new_level - level) + (1.0 - beta) * trend;
        season[si] = gamma * (yt - new_level) + (1.0 - gamma) * season[si];
        level = new_level;
        trend = new_trend;
    }
    let forecasts = (1..=horizon)
        .map(|h| level + h as f64 * trend + season[(n + h - 1) % m])
        .collect();
    let sigma = if cnt > 0.0 { (sse / cnt).sqrt() } else { 0.0 };
    (forecasts, sigma)
}

/// Holt's linear-trend smoothing (no seasonality).
fn holt_linear(y: &[f64], alpha: f64, beta: f64, horizon: usize) -> (Vec<f64>, f64) {
    let n = y.len();
    let mut level = y[0];
    let mut trend = y[1] - y[0];
    let mut sse = 0.0;
    let mut cnt = 0.0;
    for (t, &yt) in y.iter().enumerate() {
        let pred = level + trend;
        if t >= 1 {
            sse += (yt - pred).powi(2);
            cnt += 1.0;
        }
        let new_level = alpha * yt + (1.0 - alpha) * (level + trend);
        let new_trend = beta * (new_level - level) + (1.0 - beta) * trend;
        level = new_level;
        trend = new_trend;
    }
    let forecasts = (1..=horizon).map(|h| level + h as f64 * trend).collect();
    let sigma = if cnt > 0.0 { (sse / cnt).sqrt() } else { 0.0 };
    let _ = n;
    (forecasts, sigma)
}

const ALPHAS: &[f64] = &[0.1, 0.3, 0.5, 0.7, 0.9];
const BETAS: &[f64] = &[0.05, 0.1, 0.2, 0.4];
const GAMMAS: &[f64] = &[0.1, 0.3, 0.5];

/// Fit the best model for `y` and forecast `horizon` steps. Picks Holt-Winters when
/// a usable season length is given (and enough data), else Holt's linear trend, else
/// a naive last-value carry-forward. Smoothing constants chosen by a small grid
/// search minimizing in-sample residual std.
fn forecast(y: &[f64], horizon: usize, season: Option<usize>) -> Fit {
    let n = y.len();
    // Holt-Winters when a season length ≥2 is given and we have ≥2 full periods.
    if let Some(m) = season {
        if m >= 2 && n >= 2 * m {
            let mut best: Option<(f64, f64, f64, f64, Vec<f64>)> = None;
            for &a in ALPHAS {
                for &b in BETAS {
                    for &g in GAMMAS {
                        let (f, s) = holt_winters(y, m, a, b, g, horizon);
                        if best.as_ref().is_none_or(|(bs, ..)| s < *bs) {
                            best = Some((s, a, b, g, f));
                        }
                    }
                }
            }
            let (sigma, a, b, g, forecasts) = best.unwrap();
            return Fit {
                forecasts,
                sigma,
                method: format!(
                    "Holt-Winters additive (season={m}, α={}, β={}, γ={})",
                    num(a),
                    num(b),
                    num(g)
                ),
            };
        }
    }
    // Holt's linear trend for a reasonable-length series.
    if n >= 3 {
        let mut best: Option<(f64, f64, f64, Vec<f64>)> = None;
        for &a in ALPHAS {
            for &b in BETAS {
                let (f, s) = holt_linear(y, a, b, horizon);
                if best.as_ref().is_none_or(|(bs, ..)| s < *bs) {
                    best = Some((s, a, b, f));
                }
            }
        }
        let (sigma, a, b, forecasts) = best.unwrap();
        return Fit {
            forecasts,
            sigma,
            method: format!("Holt linear trend (α={}, β={})", num(a), num(b)),
        };
    }
    // Too short to model trend: carry the last value forward.
    let last = y[n - 1];
    let mean = y.iter().sum::<f64>() / n as f64;
    let var = y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
    Fit {
        forecasts: vec![last; horizon],
        sigma: var.sqrt(),
        method: "naive (last value — series too short to fit a trend)".to_string(),
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ForecastArgs {
    /// The numeric series in time order (oldest first). At least 2 points.
    values: Vec<f64>,
    /// How many future steps to forecast (1–500).
    horizon: usize,
    /// Optional season length in samples (e.g. 12 for monthly data with a yearly
    /// cycle, 7 for daily data with a weekly cycle). Needs ≥2 full periods of data;
    /// omit for a non-seasonal (trend-only) forecast.
    #[serde(default)]
    season_length: Option<usize>,
}

pub struct Forecast;
impl Skill for Forecast {
    fn name(&self) -> &'static str {
        "forecast"
    }
    fn description(&self) -> &'static str {
        "Forecast a numeric time series (local, no network). Exponential smoothing with trend and \
        optional seasonality (Holt / Holt-Winters additive); returns per-step point forecasts plus \
        an approximate ~95% interval. Give `values` (oldest first), a `horizon`, and optionally a \
        `season_length` (e.g. 12 monthly, 7 daily). A pragmatic single-binary approximation of \
        Prophet/SARIMAX, not a full statistical model."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ForecastArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ForecastArgs>()?;
            let y = args.values;
            if y.len() < 2 {
                return Err(invalid("need at least 2 data points to forecast"));
            }
            if y.iter().any(|v| !v.is_finite()) {
                return Err(invalid("values must all be finite numbers"));
            }
            let horizon = args.horizon;
            if !(1..=500).contains(&horizon) {
                return Err(invalid("horizon must be between 1 and 500"));
            }
            if let Some(m) = args.season_length {
                if m >= 2 && y.len() < 2 * m {
                    return Err(invalid(format!(
                        "season_length={m} needs at least {} data points ({} given)",
                        2 * m,
                        y.len()
                    )));
                }
            }

            let fit = forecast(&y, horizon, args.season_length);
            let mut lines = vec![
                format!("Forecast — {}", fit.method),
                format!(
                    "in-sample residual σ ≈ {} (interval ≈ point ± 1.96·σ·√h)",
                    num(fit.sigma)
                ),
                "  h   forecast        ~95% interval".to_string(),
            ];
            for (i, &f) in fit.forecasts.iter().enumerate() {
                let h = i + 1;
                let band = 1.96 * fit.sigma * (h as f64).sqrt();
                lines.push(format!(
                    "  {:<3} {:<14} [{}, {}]",
                    h,
                    num(f),
                    num(f - band),
                    num(f + band)
                ));
            }
            Ok(text_result(lines.join("\n")))
        })
    }
}

/// Local, always-on (still gateable via `[tools]`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(Forecast)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continues_a_linear_trend() {
        // y = 2t + 1 → next values should keep rising by ~2 per step.
        let y: Vec<f64> = (0..12).map(|t| 2.0 * t as f64 + 1.0).collect();
        let fit = forecast(&y, 3, None);
        assert!(fit.method.starts_with("Holt linear"));
        // Last value is 23; one step ahead should be ≈25, three ahead ≈29.
        assert!((fit.forecasts[0] - 25.0).abs() < 1.0, "{:?}", fit.forecasts);
        assert!((fit.forecasts[2] - 29.0).abs() < 1.5, "{:?}", fit.forecasts);
    }

    #[test]
    fn captures_seasonality() {
        // A clean repeating season of period 4 with no trend.
        let pattern = [10.0, 20.0, 15.0, 5.0];
        let y: Vec<f64> = (0..16).map(|t| pattern[t % 4]).collect();
        let fit = forecast(&y, 4, Some(4));
        assert!(fit.method.starts_with("Holt-Winters"));
        // The next 4 forecasts should resemble the repeating pattern.
        for (i, &p) in pattern.iter().enumerate() {
            assert!(
                (fit.forecasts[i] - p).abs() < 2.0,
                "step {i}: got {} want ≈{p}",
                fit.forecasts[i]
            );
        }
    }

    #[test]
    fn short_series_falls_back_to_naive() {
        let y = vec![5.0, 7.0];
        let fit = forecast(&y, 3, None);
        assert!(fit.method.starts_with("naive"));
        assert_eq!(fit.forecasts, vec![7.0, 7.0, 7.0]);
    }

    #[test]
    fn forecast_count_matches_horizon() {
        let y: Vec<f64> = (0..20).map(|t| (t as f64).sin() + t as f64).collect();
        assert_eq!(forecast(&y, 7, None).forecasts.len(), 7);
    }
}
