//! Time-series forecasting (local, no network, no dependency): exponential smoothing.
//!
//! Each **method is its own tool** — no hidden auto-selection: `forecast_holt_linear`
//! (Holt's linear trend) and `forecast_holt_winters` (additive trend + seasonality).
//! Smoothing constants can be pinned explicitly or, if omitted, are grid-searched to
//! minimize in-sample one-step error. A rough widening prediction interval comes from
//! the residual spread. A pragmatic single-binary stand-in for Prophet/SARIMAX.

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

/// Per-step forecasts, the in-sample residual std (for intervals), and a label.
struct Fit {
    forecasts: Vec<f64>,
    sigma: f64,
    method: String,
}

const ALPHAS: &[f64] = &[0.1, 0.3, 0.5, 0.7, 0.9];
const BETAS: &[f64] = &[0.05, 0.1, 0.2, 0.4];
const GAMMAS: &[f64] = &[0.1, 0.3, 0.5];

/// Holt-Winters **additive** smoothing: horizon forecasts + in-sample residual std.
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
    (forecasts, sigma)
}

/// Fit Holt's linear trend, pinning α/β when given, else grid-searching them.
fn fit_holt_linear(y: &[f64], horizon: usize, alpha: Option<f64>, beta: Option<f64>) -> Fit {
    let alphas = alpha.map(|a| vec![a]).unwrap_or_else(|| ALPHAS.to_vec());
    let betas = beta.map(|b| vec![b]).unwrap_or_else(|| BETAS.to_vec());
    let mut best: Option<(f64, f64, f64, Vec<f64>)> = None;
    for &a in &alphas {
        for &b in &betas {
            let (f, s) = holt_linear(y, a, b, horizon);
            if best.as_ref().is_none_or(|(bs, ..)| s < *bs) {
                best = Some((s, a, b, f));
            }
        }
    }
    let (sigma, a, b, forecasts) = best.unwrap();
    Fit {
        forecasts,
        sigma,
        method: format!("Holt linear trend (α={}, β={})", num(a), num(b)),
    }
}

/// Fit Holt-Winters additive, pinning α/β/γ when given, else grid-searching them.
fn fit_holt_winters(
    y: &[f64],
    m: usize,
    horizon: usize,
    alpha: Option<f64>,
    beta: Option<f64>,
    gamma: Option<f64>,
) -> Fit {
    let alphas = alpha.map(|a| vec![a]).unwrap_or_else(|| ALPHAS.to_vec());
    let betas = beta.map(|b| vec![b]).unwrap_or_else(|| BETAS.to_vec());
    let gammas = gamma.map(|g| vec![g]).unwrap_or_else(|| GAMMAS.to_vec());
    let mut best: Option<(f64, f64, f64, f64, Vec<f64>)> = None;
    for &a in &alphas {
        for &b in &betas {
            for &g in &gammas {
                let (f, s) = holt_winters(y, m, a, b, g, horizon);
                if best.as_ref().is_none_or(|(bs, ..)| s < *bs) {
                    best = Some((s, a, b, g, f));
                }
            }
        }
    }
    let (sigma, a, b, g, forecasts) = best.unwrap();
    Fit {
        forecasts,
        sigma,
        method: format!(
            "Holt-Winters additive (season={m}, α={}, β={}, γ={})",
            num(a),
            num(b),
            num(g)
        ),
    }
}

/// Render a fit as point forecasts + an approximate ~95% interval.
fn render(fit: &Fit) -> String {
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
    lines.join("\n")
}

/// Validate the shared `values` + `horizon` inputs.
fn check(values: &[f64], horizon: usize) -> Result<(), McpError> {
    if values.len() < 2 {
        return Err(invalid("need at least 2 data points to forecast"));
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Err(invalid("values must all be finite numbers"));
    }
    if !(1..=500).contains(&horizon) {
        return Err(invalid("horizon must be between 1 and 500"));
    }
    Ok(())
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HoltLinearArgs {
    /// The numeric series in time order (oldest first). At least 2 points.
    values: Vec<f64>,
    /// How many future steps to forecast (1–500).
    horizon: usize,
    /// Level smoothing α (0–1). Omit to grid-search the best fit.
    #[serde(default)]
    alpha: Option<f64>,
    /// Trend smoothing β (0–1). Omit to grid-search the best fit.
    #[serde(default)]
    beta: Option<f64>,
}

pub struct ForecastHoltLinear;
impl Skill for ForecastHoltLinear {
    fn name(&self) -> &'static str {
        "forecast_holt_linear"
    }
    fn description(&self) -> &'static str {
        "Forecast a numeric series with Holt's linear-trend exponential smoothing (level + trend, \
        no seasonality), local. Give `values` (oldest first) and a `horizon`; optionally pin the \
        smoothing constants `alpha`/`beta` (else they're grid-searched). Returns per-step forecasts \
        with an approximate ~95% interval. For a seasonal series use forecast_holt_winters."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HoltLinearArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<HoltLinearArgs>()?;
            check(&args.values, args.horizon)?;
            for (label, p) in [("alpha", args.alpha), ("beta", args.beta)] {
                if let Some(p) = p {
                    if !(0.0..=1.0).contains(&p) {
                        return Err(invalid(format!("{label} must be between 0 and 1")));
                    }
                }
            }
            let fit = fit_holt_linear(&args.values, args.horizon, args.alpha, args.beta);
            Ok(text_result(render(&fit)))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Forecast 3 steps, grid-search params",
                args: r#"{"values": [1.0, 3.0, 5.0, 7.0, 9.0, 11.0], "horizon": 3}"#,
                note: Some("Omit α/β to grid-search the best in-sample fit."),
            },
            SkillExample {
                title: "Pin alpha and beta",
                args: r#"{"values": [10.0, 12.0, 14.0, 16.0, 18.0], "horizon": 2, "alpha": 0.5, "beta": 0.1}"#,
                note: Some("Useful when you've already calibrated the smoothing constants."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Forecast a trending series with no clear seasonality, locally.",
            "Drop-in trend forecast when you don't want to depend on Prophet / SARIMAX.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HoltWintersArgs {
    /// The numeric series in time order (oldest first). Needs ≥2 full seasons.
    values: Vec<f64>,
    /// How many future steps to forecast (1–500).
    horizon: usize,
    /// Season length in samples (e.g. 12 monthly w/ yearly cycle, 7 daily w/ weekly).
    season_length: usize,
    /// Level smoothing α (0–1). Omit to grid-search.
    #[serde(default)]
    alpha: Option<f64>,
    /// Trend smoothing β (0–1). Omit to grid-search.
    #[serde(default)]
    beta: Option<f64>,
    /// Seasonal smoothing γ (0–1). Omit to grid-search.
    #[serde(default)]
    gamma: Option<f64>,
}

pub struct ForecastHoltWinters;
impl Skill for ForecastHoltWinters {
    fn name(&self) -> &'static str {
        "forecast_holt_winters"
    }
    fn description(&self) -> &'static str {
        "Forecast a SEASONAL numeric series with Holt-Winters additive exponential smoothing (level \
        + trend + seasonality), local. Give `values` (oldest first), a `horizon`, and a \
        `season_length` (e.g. 12 monthly, 7 daily); needs ≥2 full seasons. Optionally pin \
        `alpha`/`beta`/`gamma` (else grid-searched). Returns per-step forecasts + an approximate \
        interval. For a non-seasonal series use forecast_holt_linear."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HoltWintersArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<HoltWintersArgs>()?;
            check(&args.values, args.horizon)?;
            let m = args.season_length;
            if m < 2 {
                return Err(invalid("season_length must be at least 2"));
            }
            if args.values.len() < 2 * m {
                return Err(invalid(format!(
                    "Holt-Winters needs at least {} data points (2 full seasons of {m}); {} given",
                    2 * m,
                    args.values.len()
                )));
            }
            for (label, p) in [
                ("alpha", args.alpha),
                ("beta", args.beta),
                ("gamma", args.gamma),
            ] {
                if let Some(p) = p {
                    if !(0.0..=1.0).contains(&p) {
                        return Err(invalid(format!("{label} must be between 0 and 1")));
                    }
                }
            }
            let fit = fit_holt_winters(
                &args.values,
                m,
                args.horizon,
                args.alpha,
                args.beta,
                args.gamma,
            );
            Ok(text_result(render(&fit)))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Quarterly cycle, 4 steps ahead",
                args: r#"{"values": [10.0, 20.0, 15.0, 5.0, 10.0, 20.0, 15.0, 5.0], "horizon": 4, "season_length": 4}"#,
                note: Some("Needs ≥2 full seasons; grid-searches α/β/γ."),
            },
            SkillExample {
                title: "Weekly seasonality, pinned params",
                args: r#"{"values": [1, 2, 3, 4, 5, 6, 7, 1.1, 2.1, 3.1, 4.1, 5.1, 6.1, 7.1], "horizon": 7, "season_length": 7, "alpha": 0.3, "beta": 0.1, "gamma": 0.3}"#,
                note: Some("Pin smoothing constants once you trust them."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Forecast a series with additive trend AND repeating seasonality.",
            "Use when you have ≥2 full seasons; for non-seasonal data prefer forecast_holt_linear.",
        ]
    }
}

/// Local, always-on (still gateable via `[tools]`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(ForecastHoltLinear), Box::new(ForecastHoltWinters)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holt_linear_continues_trend() {
        let y: Vec<f64> = (0..12).map(|t| 2.0 * t as f64 + 1.0).collect();
        let fit = fit_holt_linear(&y, 3, None, None);
        assert!(fit.method.starts_with("Holt linear"));
        assert!((fit.forecasts[0] - 25.0).abs() < 1.0, "{:?}", fit.forecasts);
        assert!((fit.forecasts[2] - 29.0).abs() < 1.5, "{:?}", fit.forecasts);
    }

    #[test]
    fn holt_winters_captures_season() {
        let pattern = [10.0, 20.0, 15.0, 5.0];
        let y: Vec<f64> = (0..16).map(|t| pattern[t % 4]).collect();
        let fit = fit_holt_winters(&y, 4, 4, None, None, None);
        assert!(fit.method.starts_with("Holt-Winters"));
        for (i, &p) in pattern.iter().enumerate() {
            assert!(
                (fit.forecasts[i] - p).abs() < 2.0,
                "step {i}: {}",
                fit.forecasts[i]
            );
        }
    }

    #[test]
    fn explicit_params_are_honored() {
        let y: Vec<f64> = (0..10).map(|t| t as f64).collect();
        let fit = fit_holt_linear(&y, 1, Some(0.5), Some(0.1));
        assert!(fit.method.contains("α=0.5") && fit.method.contains("β=0.1"));
    }

    #[test]
    fn forecast_count_matches_horizon() {
        let y: Vec<f64> = (0..20).map(|t| (t as f64).sin() + t as f64).collect();
        assert_eq!(fit_holt_linear(&y, 7, None, None).forecasts.len(), 7);
    }
}
