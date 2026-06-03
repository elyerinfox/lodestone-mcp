//! Descriptive statistics skills (local compute): summary stats,
//! percentiles, correlation, z-scores. Pure-Rust, no external crate.
//! LLMs do small arithmetic mistakes on means / standard deviations
//! and frequently mix up population vs sample variance.
//!
//! ## Sources
//!
//! - Welford 1962 (online stable variance — used here so a long input
//!   list doesn't lose precision).
//! - NIST/SEMATECH e-Handbook of Statistical Methods.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DataArgs {
    /// Numeric sample. At least one value required.
    data: Vec<f64>,
}

pub struct StatsSummary;
impl Skill for StatsSummary {
    fn name(&self) -> &'static str {
        "stats_summary"
    }
    fn description(&self) -> &'static str {
        "Descriptive statistics for a numeric sample: count, min, max, mean, median, mode (if a value repeats), variance and standard deviation (BOTH sample and population), range, interquartile range. Welford-stable mean/variance."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DataArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DataArgs>()?;
            if a.data.is_empty() {
                return Err(invalid("data must contain at least one value"));
            }
            let n = a.data.len();
            let (mean, var_pop, var_sample) = welford(&a.data);
            let std_pop = var_pop.sqrt();
            let std_sample = var_sample.sqrt();
            let mut sorted = a.data.clone();
            sorted.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let min = sorted[0];
            let max = sorted[n - 1];
            let median = median_of(&sorted);
            let q1 = percentile_of(&sorted, 25.0);
            let q3 = percentile_of(&sorted, 75.0);
            let iqr = q3 - q1;
            let mode = compute_mode(&a.data);
            Ok(text_result(
                json!({
                    "n": n,
                    "min": min,
                    "max": max,
                    "range": max - min,
                    "mean": mean,
                    "median": median,
                    "mode": mode,
                    "variance_population": var_pop,
                    "variance_sample": var_sample,
                    "std_population": std_pop,
                    "std_sample": std_sample,
                    "q1": q1,
                    "q3": q3,
                    "iqr": iqr,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Simple list",
                args: r#"{"data": [2, 4, 4, 4, 5, 5, 7, 9]}"#,
                note: Some("Returns mean=5, sample stdev≈2.138, population stdev=2."),
            },
            SkillExample {
                title: "Long input",
                args: r#"{"data": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]}"#,
                note: Some("Median = 5.5; IQR = 4.5."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute the right stdev (sample vs population) without picking the wrong divisor.",
            "Get a precise five-number summary for a list.",
        ]
    }
}

fn welford(data: &[f64]) -> (f64, f64, f64) {
    let mut mean = 0.0;
    let mut m2 = 0.0;
    for (i, x) in data.iter().enumerate() {
        let count = (i + 1) as f64;
        let delta = x - mean;
        mean += delta / count;
        let delta2 = x - mean;
        m2 += delta * delta2;
    }
    let n = data.len() as f64;
    let var_pop = m2 / n;
    let var_sample = if n > 1.0 { m2 / (n - 1.0) } else { 0.0 };
    (mean, var_pop, var_sample)
}

fn median_of(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

fn percentile_of(sorted: &[f64], p: f64) -> f64 {
    // Linear interpolation between closest ranks (R type 7 — Excel / numpy default).
    if sorted.is_empty() {
        return f64::NAN;
    }
    let n = sorted.len();
    if n == 1 {
        return sorted[0];
    }
    let h = (p / 100.0) * (n - 1) as f64;
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    let frac = h - lo as f64;
    sorted[lo] + frac * (sorted[hi] - sorted[lo])
}

fn compute_mode(data: &[f64]) -> serde_json::Value {
    use std::collections::HashMap;
    // Quantize to bit-pattern keys (treat -0.0 == 0.0 by canonicalizing).
    let mut counts: HashMap<u64, (f64, usize)> = HashMap::new();
    for &x in data {
        let x_canon = if x == 0.0 { 0.0 } else { x };
        let bits = x_canon.to_bits();
        let e = counts.entry(bits).or_insert((x_canon, 0));
        e.1 += 1;
    }
    let max = counts.values().map(|(_, c)| *c).max().unwrap_or(0);
    if max == 1 {
        return serde_json::Value::Null;
    } // no real mode
    let modes: Vec<f64> = counts
        .values()
        .filter(|(_, c)| *c == max)
        .map(|(v, _)| *v)
        .collect();
    if modes.len() == 1 {
        json!(modes[0])
    } else {
        let mut sorted = modes;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        json!(sorted)
    }
}

// ---------- percentile ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PercArgs {
    /// Numeric sample.
    data: Vec<f64>,
    /// Percentile rank (0..=100).
    p: f64,
}

pub struct StatsPercentile;
impl Skill for StatsPercentile {
    fn name(&self) -> &'static str {
        "stats_percentile"
    }
    fn description(&self) -> &'static str {
        "p-th percentile of a sample via linear interpolation (R-7 / numpy default). p must be in 0..=100."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PercArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PercArgs>()?;
            if a.data.is_empty() {
                return Err(invalid("data must contain at least one value"));
            }
            if !(0.0..=100.0).contains(&a.p) {
                return Err(invalid("p must be in 0..=100"));
            }
            let mut sorted = a.data.clone();
            sorted.sort_by(|x, y| x.partial_cmp(y).unwrap());
            let v = percentile_of(&sorted, a.p);
            Ok(text_result(json!({"p": a.p, "value": v}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "p90",
                args: r#"{"data": [1,2,3,4,5,6,7,8,9,10], "p": 90}"#,
                note: Some("Returns `9.1`."),
            },
            SkillExample {
                title: "Median (p50)",
                args: r#"{"data": [3,1,4,1,5,9,2,6], "p": 50}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Compute a single percentile (p95 latency, p99, etc.) accurately.", "Use the R-7 / numpy-equivalent linear interpolation method without re-implementing it."]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range { field: "p", min: Some(0.0), max: Some(100.0) },
            Rule::Length { field: "data", min: Some(1), max: None },
        ]
    }
}

// ---------- correlation ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CorrArgs {
    /// First sample.
    x: Vec<f64>,
    /// Second sample, same length as `x`.
    y: Vec<f64>,
}

pub struct StatsCorrelation;
impl Skill for StatsCorrelation {
    fn name(&self) -> &'static str {
        "stats_correlation"
    }
    fn description(&self) -> &'static str {
        "Pearson product-moment correlation r between two equal-length samples. Returns r plus the coefficient of determination r^2. NaN if one sample has zero variance."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CorrArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CorrArgs>()?;
            if a.x.is_empty() || a.y.is_empty() {
                return Err(invalid("samples cannot be empty"));
            }
            if a.x.len() != a.y.len() {
                return Err(invalid("x and y must be the same length"));
            }
            let r = pearson(&a.x, &a.y);
            Ok(text_result(
                json!({"r": r, "r2": r * r, "n": a.x.len()}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Perfectly correlated",
                args: r#"{"x": [1,2,3,4,5], "y": [2,4,6,8,10]}"#,
                note: Some("r = 1.0."),
            },
            SkillExample {
                title: "No correlation",
                args: r#"{"x": [1,2,3,4,5], "y": [3,1,4,1,5]}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Quantify the linear relationship between two metrics.",
            "Confirm a hand-calculated r without arithmetic drift.",
        ]
    }
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let mean_x = x.iter().sum::<f64>() / n;
    let mean_y = y.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut dx2 = 0.0;
    let mut dy2 = 0.0;
    for i in 0..x.len() {
        let dx = x[i] - mean_x;
        let dy = y[i] - mean_y;
        num += dx * dy;
        dx2 += dx * dx;
        dy2 += dy * dy;
    }
    let denom = (dx2 * dy2).sqrt();
    if denom == 0.0 {
        f64::NAN
    } else {
        num / denom
    }
}

// ---------- z-score ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ZArgs {
    /// Value to standardize.
    value: f64,
    /// Sample to compute the z-score against (at least 2 values).
    data: Vec<f64>,
}

pub struct StatsZScore;
impl Skill for StatsZScore {
    fn name(&self) -> &'static str {
        "stats_zscore"
    }
    fn description(&self) -> &'static str {
        "z-score of `value` relative to `data`: (value - mean) / sample_stdev. Sample stdev (n-1 divisor) is the standard choice for outlier detection."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ZArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ZArgs>()?;
            if a.data.len() < 2 {
                return Err(invalid("data must have at least 2 values for sample stdev"));
            }
            let (mean, _, var_sample) = welford(&a.data);
            let sd = var_sample.sqrt();
            let z = if sd == 0.0 {
                f64::NAN
            } else {
                (a.value - mean) / sd
            };
            Ok(text_result(
                json!({"value": a.value, "mean": mean, "stdev_sample": sd, "z": z}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Outlier check",
            args: r#"{"value": 100, "data": [10,12,11,13,12,11,10,9]}"#,
            note: Some("Large |z| indicates outlier."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decide whether a single observation is an outlier in a sample.",
            "Convert raw measurements to standardized scores for cross-comparison.",
        ]
    }
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "stats"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Descriptive statistics: summary, percentile, Pearson correlation, z-score. Welford-stable. Pure local compute."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some("1. `stats_summary { data: [...] }` — full descriptive view.\n2. `stats_percentile { data: [...], p: 95 }` — single percentile.\n3. `stats_zscore { value: X, data: [...] }` — outlier check.")
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(StatsSummary),
        Box::new(StatsPercentile),
        Box::new(StatsCorrelation),
        Box::new(StatsZScore),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn welford_matches_naive() {
        let d = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let (mean, var_pop, _) = welford(&d);
        assert!((mean - 5.0).abs() < 1e-9);
        assert!((var_pop - 4.0).abs() < 1e-9);
    }
    #[test]
    fn median_odd() {
        assert_eq!(median_of(&[1.0, 2.0, 3.0]), 2.0);
    }
    #[test]
    fn median_even() {
        assert_eq!(median_of(&[1.0, 2.0, 3.0, 4.0]), 2.5);
    }
    #[test]
    fn percentile_p50_is_median() {
        let s = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile_of(&s, 50.0), 5.5);
    }
    #[test]
    fn pearson_perfect() {
        assert!(
            (pearson(&[1.0, 2.0, 3.0, 4.0, 5.0], &[2.0, 4.0, 6.0, 8.0, 10.0]) - 1.0).abs() < 1e-9
        );
    }
}
