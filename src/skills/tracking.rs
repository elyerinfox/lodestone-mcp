//! Estimation and tracking primitives — Kalman family, RANSAC, Hungarian
//! assignment. On by default. The KF tools are single-step (predict +
//! update) so the model drives them across calls; storing state across
//! calls would belong on the TaskRuntime as a higher-level orchestration.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use nalgebra::{DMatrix, DVector};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

fn rows_to_dmatrix(rows: &[Vec<f64>]) -> std::result::Result<DMatrix<f64>, McpError> {
    if rows.is_empty() {
        return Err(invalid("matrix empty"));
    }
    let n = rows[0].len();
    if n == 0 {
        return Err(invalid("matrix has zero cols"));
    }
    for r in rows {
        if r.len() != n {
            return Err(invalid("matrix is ragged"));
        }
    }
    let flat: Vec<f64> = rows.iter().flatten().copied().collect();
    Ok(DMatrix::from_row_slice(rows.len(), n, &flat))
}

fn vec_to_dvec(v: &[f64]) -> std::result::Result<DVector<f64>, McpError> {
    if v.is_empty() {
        return Err(invalid("vector empty"));
    }
    Ok(DVector::from_column_slice(v))
}

fn mat_to_rows(m: &DMatrix<f64>) -> Vec<Vec<f64>> {
    (0..m.nrows())
        .map(|i| (0..m.ncols()).map(|j| m[(i, j)]).collect())
        .collect()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct KfArgs {
    /// State estimate x (n).
    x: Vec<f64>,
    /// State covariance P (n × n).
    p: Vec<Vec<f64>>,
    /// State transition F (n × n).
    f: Vec<Vec<f64>>,
    /// Process noise Q (n × n).
    q: Vec<Vec<f64>>,
    /// Measurement model H (m × n).
    h: Vec<Vec<f64>>,
    /// Measurement noise R (m × m).
    r: Vec<Vec<f64>>,
    /// Measurement z (m).
    z: Vec<f64>,
}

pub struct TrackKalman;
impl Skill for TrackKalman {
    fn name(&self) -> &'static str {
        "track_kalman_step"
    }
    fn description(&self) -> &'static str {
        "Single linear Kalman filter step (predict + update). Returns the \
        posterior state `x` + covariance `p`, the innovation `y`, and the \
        normalized innovation squared (NIS = yᵀ S⁻¹ y) for chi-squared \
        gating. The model supplies F, Q, H, R every call; for time-varying \
        models pass the relevant matrices each step."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<KfArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<KfArgs>()?;
            let x = vec_to_dvec(&a.x)?;
            let p = rows_to_dmatrix(&a.p)?;
            let f = rows_to_dmatrix(&a.f)?;
            let q = rows_to_dmatrix(&a.q)?;
            let h = rows_to_dmatrix(&a.h)?;
            let r = rows_to_dmatrix(&a.r)?;
            let z = vec_to_dvec(&a.z)?;

            // Predict.
            let x_pred = &f * &x;
            let p_pred = &f * &p * f.transpose() + &q;

            // Update.
            let y = &z - &h * &x_pred;
            let s = &h * &p_pred * h.transpose() + &r;
            let s_inv = s
                .clone()
                .try_inverse()
                .ok_or_else(|| invalid("innovation covariance singular"))?;
            let k = &p_pred * h.transpose() * &s_inv;
            let x_post = &x_pred + &k * &y;
            let i_n = DMatrix::<f64>::identity(p.nrows(), p.nrows());
            let p_post = (&i_n - &k * &h) * &p_pred;

            let nis = (y.transpose() * s_inv * &y)[(0, 0)];

            Ok(text_result(
                json!({
                    "x": x_post.as_slice().to_vec(),
                    "p": mat_to_rows(&p_post),
                    "innovation": y.as_slice().to_vec(),
                    "nis": nis,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Constant-velocity 1-D, single update",
                args: r#"{"x": [0.0, 1.0], "p": [[1.0, 0.0], [0.0, 1.0]], "f": [[1.0, 1.0], [0.0, 1.0]], "q": [[0.01, 0.0], [0.0, 0.01]], "h": [[1.0, 0.0]], "r": [[0.5]], "z": [1.2]}"#,
                note: Some("Returns posterior x, P, innovation y, and NIS."),
            },
            SkillExample {
                title: "Position-only observation",
                args: r#"{"x": [10.0, 0.0], "p": [[4.0, 0.0], [0.0, 1.0]], "f": [[1.0, 0.1], [0.0, 1.0]], "q": [[0.001, 0.0], [0.0, 0.001]], "h": [[1.0, 0.0]], "r": [[1.0]], "z": [10.5]}"#,
                note: Some("Use NIS for chi-squared gating across multiple steps."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Run one predict+update step of a linear Kalman filter from caller-supplied matrices.",
            "Drive a stateless KF where the orchestration layer carries x and P between calls.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HungarianArgs {
    /// Cost matrix (workers × jobs).
    cost: Vec<Vec<f64>>,
}

pub struct TrackHungarian;
impl Skill for TrackHungarian {
    fn name(&self) -> &'static str {
        "track_hungarian"
    }
    fn description(&self) -> &'static str {
        "Hungarian / Kuhn-Munkres optimal assignment minimizing total cost \
        on a non-negative rectangular cost matrix. Returns `assignment` \
        (worker_i → job_j or -1 if unassigned) and the total cost."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HungarianArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<HungarianArgs>()?;
            if a.cost.is_empty() {
                return Err(invalid("cost matrix empty"));
            }
            // Pathfinding's kuhn_munkres needs i32 weights minimizing — supply integer scale.
            let scale = 1e6;
            let n = a.cost.len();
            let m = a.cost[0].len();
            for r in &a.cost {
                if r.len() != m {
                    return Err(invalid("cost matrix is ragged"));
                }
            }
            // Square it up by padding with a large constant.
            let dim = n.max(m);
            let mut sq = vec![vec![0_i64; dim]; dim];
            let big = 1e15 as i64;
            for (i, row) in sq.iter_mut().enumerate().take(dim) {
                for (j, cell) in row.iter_mut().enumerate().take(dim) {
                    *cell = if i < n && j < m {
                        (a.cost[i][j] * scale).round() as i64
                    } else {
                        big
                    };
                }
            }
            let mat = pathfinding::matrix::Matrix::from_rows(sq.iter().cloned())
                .map_err(|e| invalid(format!("hungarian matrix: {e}")))?;
            // kuhn_munkres MAXIMIZES; negate for min.
            let neg = mat.map(|v: i64| -v);
            let (_total, assign) = pathfinding::kuhn_munkres::kuhn_munkres(&neg);
            let mut out: Vec<i64> = vec![-1; n];
            let mut total = 0.0_f64;
            for (i, j) in assign.iter().enumerate() {
                if i < n && *j < m {
                    out[i] = *j as i64;
                    total += a.cost[i][*j];
                }
            }
            Ok(text_result(
                json!({ "assignment": out, "total_cost": total }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "3×3 square cost",
                args: r#"{"cost": [[4.0, 1.0, 3.0], [2.0, 0.0, 5.0], [3.0, 2.0, 2.0]]}"#,
                note: Some("Returns assignment worker_i → job_j minimizing total cost."),
            },
            SkillExample {
                title: "Rectangular, more workers than jobs",
                args: r#"{"cost": [[1.0, 2.0], [3.0, 1.0], [2.0, 5.0]]}"#,
                note: Some("Padded internally; extra workers get assignment = -1."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Assign tracks ↔ detections in a multi-target tracker minimizing total gating cost.",
            "Solve any min-cost bipartite assignment with non-negative weights.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RansacLineArgs {
    /// 2-D points to fit a line through.
    points: Vec<[f64; 2]>,
    /// Inlier distance threshold.
    threshold: f64,
    /// Iterations (default 200).
    #[serde(default)]
    iterations: Option<usize>,
}

pub struct TrackRansacLine;
impl Skill for TrackRansacLine {
    fn name(&self) -> &'static str {
        "track_ransac_line"
    }
    fn description(&self) -> &'static str {
        "RANSAC line fit on 2-D points. Returns line coefficients (a, b, c) \
        for ax + by + c = 0 normalized, the inlier indices, and the inlier \
        count. Robust to outliers up to ~50 % contamination."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RansacLineArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use rand::seq::SliceRandom;
            use rand::thread_rng;
            let (_s, a) = ctx.parse::<RansacLineArgs>()?;
            if a.points.len() < 2 {
                return Err(invalid("need ≥ 2 points"));
            }
            let iters = a.iterations.unwrap_or(200);
            let mut rng = thread_rng();
            let mut best_inliers: Vec<usize> = Vec::new();
            let mut best_abc = (0.0_f64, 0.0_f64, 0.0_f64);
            let pts: Vec<usize> = (0..a.points.len()).collect();
            for _ in 0..iters {
                let sample: Vec<&usize> = pts.choose_multiple(&mut rng, 2).collect();
                let p1 = a.points[*sample[0]];
                let p2 = a.points[*sample[1]];
                let (a_c, b_c, c_c) = normalize_line(p1[0], p1[1], p2[0], p2[1]);
                if a_c.is_nan() {
                    continue;
                }
                let mut inliers = Vec::new();
                for (i, p) in a.points.iter().enumerate() {
                    let d = (a_c * p[0] + b_c * p[1] + c_c).abs();
                    if d <= a.threshold {
                        inliers.push(i);
                    }
                }
                if inliers.len() > best_inliers.len() {
                    best_inliers = inliers;
                    best_abc = (a_c, b_c, c_c);
                }
            }
            Ok(text_result(
                json!({
                    "a": best_abc.0,
                    "b": best_abc.1,
                    "c": best_abc.2,
                    "inliers": best_inliers,
                    "inlier_count": best_inliers.len(),
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Mostly-inlier line",
                args: r#"{"points": [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0], [10.0, -5.0]], "threshold": 0.1}"#,
                note: Some("Returns normalized (a, b, c) for ax+by+c=0 and the inlier indices."),
            },
            SkillExample {
                title: "More iterations for noisy data",
                args: r#"{"points": [[0.0, 0.0], [1.0, 0.9], [2.0, 2.1], [3.0, 3.0], [4.0, 4.2], [0.5, 5.0]], "threshold": 0.3, "iterations": 1000}"#,
                note: Some("Raise iterations when contamination is high."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Robustly fit a 2-D line through outlier-contaminated points.",
            "Identify which points belong to a dominant linear feature (lane edge, scan line).",
        ]
    }
}

fn normalize_line(x1: f64, y1: f64, x2: f64, y2: f64) -> (f64, f64, f64) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let n = (dx * dx + dy * dy).sqrt();
    if n == 0.0 {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    let a = -dy / n;
    let b = dx / n;
    let c = -(a * x1 + b * y1);
    (a, b, c)
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(TrackKalman),
        Box::new(TrackHungarian),
        Box::new(TrackRansacLine),
    ]
}
