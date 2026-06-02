//! Linear algebra skill — thin wrappers over `nalgebra` for the common
//! operations the tracking / nav / DSP families lean on. On by default
//! (pure math, no host requirement).
//!
//! Tools: `linalg_solve` (Ax = b), `linalg_lstsq` (least-squares),
//! `linalg_svd`, `linalg_eigen`, `linalg_qr`, `linalg_inv`, `linalg_det`,
//! `linalg_rank`, `linalg_norm`, `linalg_matmul`.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use nalgebra::{DMatrix, DVector};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// Parse a JSON 2-D array of numbers into a column-major `DMatrix<f64>`.
/// Rejects ragged rows.
fn parse_matrix(rows: &[Vec<f64>], name: &str) -> Result<DMatrix<f64>> {
    if rows.is_empty() {
        return Err(anyhow!("{name} is empty"));
    }
    let ncols = rows[0].len();
    if ncols == 0 {
        return Err(anyhow!("{name} has zero columns"));
    }
    for (i, r) in rows.iter().enumerate() {
        if r.len() != ncols {
            return Err(anyhow!(
                "{name} is ragged: row {i} has {} cols, expected {ncols}",
                r.len()
            ));
        }
    }
    let nrows = rows.len();
    let flat: Vec<f64> = (0..nrows).flat_map(|i| rows[i].clone()).collect();
    Ok(DMatrix::from_row_slice(nrows, ncols, &flat))
}

fn parse_vector(v: &[f64], name: &str) -> Result<DVector<f64>> {
    if v.is_empty() {
        return Err(anyhow!("{name} is empty"));
    }
    Ok(DVector::from_column_slice(v))
}

/// Render a matrix back into a Vec<Vec<f64>> for JSON.
fn matrix_to_rows(m: &DMatrix<f64>) -> Vec<Vec<f64>> {
    (0..m.nrows())
        .map(|i| (0..m.ncols()).map(|j| m[(i, j)]).collect())
        .collect()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MatArgs {
    /// Row-major matrix: `[[1,2,3],[4,5,6]]`.
    matrix: Vec<Vec<f64>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolveArgs {
    /// Coefficient matrix A (m × n).
    a: Vec<Vec<f64>>,
    /// Right-hand-side vector b (length m).
    b: Vec<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MatMulArgs {
    /// Left matrix A (m × k).
    a: Vec<Vec<f64>>,
    /// Right matrix B (k × n).
    b: Vec<Vec<f64>>,
}

pub struct LinalgSolve;
impl Skill for LinalgSolve {
    fn name(&self) -> &'static str {
        "linalg_solve"
    }
    fn description(&self) -> &'static str {
        "Solve Ax = b for a square A via LU decomposition. Returns `x`. \
        Errors if A is singular. For tall/wide A use `linalg_lstsq`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<SolveArgs>()?;
            let a = parse_matrix(&args.a, "a").map_err(invalid)?;
            let b = parse_vector(&args.b, "b").map_err(invalid)?;
            if a.nrows() != a.ncols() {
                return Err(invalid(
                    "linalg_solve requires a square A; use linalg_lstsq otherwise",
                ));
            }
            if a.nrows() != b.len() {
                return Err(invalid("A.nrows must equal b.len"));
            }
            let lu = a.lu();
            let x = lu
                .solve(&b)
                .ok_or_else(|| invalid("A is singular (no solution)"))?;
            Ok(text_result(
                json!({ "x": x.as_slice().to_vec() }).to_string(),
            ))
        })
    }
}

pub struct LinalgLstsq;
impl Skill for LinalgLstsq {
    fn name(&self) -> &'static str {
        "linalg_lstsq"
    }
    fn description(&self) -> &'static str {
        "Least-squares solution to Ax = b via SVD (handles tall, wide, and \
        rank-deficient A). Returns `x` minimizing ||Ax-b||₂."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<SolveArgs>()?;
            let a = parse_matrix(&args.a, "a").map_err(invalid)?;
            let b = parse_vector(&args.b, "b").map_err(invalid)?;
            if a.nrows() != b.len() {
                return Err(invalid("A.nrows must equal b.len"));
            }
            let svd = a.clone().svd(true, true);
            let x = svd
                .solve(&b, 1e-12)
                .map_err(|e| invalid(format!("least-squares failed: {e}")))?;
            // Residual norm.
            let r = (&a * &x) - &b;
            let residual = r.norm();
            Ok(text_result(
                json!({
                    "x": x.as_slice().to_vec(),
                    "residual_norm": residual,
                })
                .to_string(),
            ))
        })
    }
}

pub struct LinalgSvd;
impl Skill for LinalgSvd {
    fn name(&self) -> &'static str {
        "linalg_svd"
    }
    fn description(&self) -> &'static str {
        "Singular value decomposition A = U·Σ·Vᵀ. Returns `singular_values`, \
        `u` (left singular vectors), `v_t` (transpose of right)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<MatArgs>()?;
            let a = parse_matrix(&args.matrix, "matrix").map_err(invalid)?;
            let svd = a.svd(true, true);
            let u = svd.u.ok_or_else(|| internal(anyhow!("U not computed")))?;
            let vt = svd
                .v_t
                .ok_or_else(|| internal(anyhow!("V_t not computed")))?;
            Ok(text_result(
                json!({
                    "singular_values": svd.singular_values.as_slice().to_vec(),
                    "u": matrix_to_rows(&u),
                    "v_t": matrix_to_rows(&vt),
                })
                .to_string(),
            ))
        })
    }
}

pub struct LinalgEigen;
impl Skill for LinalgEigen {
    fn name(&self) -> &'static str {
        "linalg_eigen"
    }
    fn description(&self) -> &'static str {
        "Symmetric eigen-decomposition (input must be symmetric). Returns \
        `eigenvalues` (ascending) and `eigenvectors` (columns). For general \
        non-symmetric A use SVD or specialized solvers."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<MatArgs>()?;
            let a = parse_matrix(&args.matrix, "matrix").map_err(invalid)?;
            if a.nrows() != a.ncols() {
                return Err(invalid("eigen requires a square symmetric matrix"));
            }
            // Tolerate small asymmetry (numerical) by symmetrizing.
            let sym = (&a + a.transpose()) * 0.5;
            let eig = sym.symmetric_eigen();
            Ok(text_result(
                json!({
                    "eigenvalues": eig.eigenvalues.as_slice().to_vec(),
                    "eigenvectors": matrix_to_rows(&eig.eigenvectors),
                })
                .to_string(),
            ))
        })
    }
}

pub struct LinalgQr;
impl Skill for LinalgQr {
    fn name(&self) -> &'static str {
        "linalg_qr"
    }
    fn description(&self) -> &'static str {
        "QR decomposition A = Q·R. Returns `q` (orthonormal) and `r` (upper \
        triangular). Used inside least-squares and matrix conditioning checks."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<MatArgs>()?;
            let a = parse_matrix(&args.matrix, "matrix").map_err(invalid)?;
            let qr = a.qr();
            let q = qr.q();
            let r = qr.r();
            Ok(text_result(
                json!({ "q": matrix_to_rows(&q), "r": matrix_to_rows(&r) }).to_string(),
            ))
        })
    }
}

pub struct LinalgInv;
impl Skill for LinalgInv {
    fn name(&self) -> &'static str {
        "linalg_inv"
    }
    fn description(&self) -> &'static str {
        "Matrix inverse (requires square A). Errors if A is singular. Prefer \
        `linalg_solve` when you actually need to solve Ax = b — it's faster \
        and more accurate than computing A⁻¹·b."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<MatArgs>()?;
            let a = parse_matrix(&args.matrix, "matrix").map_err(invalid)?;
            if a.nrows() != a.ncols() {
                return Err(invalid("inverse requires a square matrix"));
            }
            let inv = a
                .try_inverse()
                .ok_or_else(|| invalid("matrix is singular"))?;
            Ok(text_result(
                json!({ "inverse": matrix_to_rows(&inv) }).to_string(),
            ))
        })
    }
}

pub struct LinalgDet;
impl Skill for LinalgDet {
    fn name(&self) -> &'static str {
        "linalg_det"
    }
    fn description(&self) -> &'static str {
        "Matrix determinant (square A only)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<MatArgs>()?;
            let a = parse_matrix(&args.matrix, "matrix").map_err(invalid)?;
            if a.nrows() != a.ncols() {
                return Err(invalid("determinant requires a square matrix"));
            }
            Ok(text_result(
                json!({ "determinant": a.determinant() }).to_string(),
            ))
        })
    }
}

pub struct LinalgRank;
impl Skill for LinalgRank {
    fn name(&self) -> &'static str {
        "linalg_rank"
    }
    fn description(&self) -> &'static str {
        "Numerical matrix rank via SVD with a tolerance — counts singular \
        values greater than `tol·max(σ)` (default tol = 1e-12)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<MatArgs>()?;
            let a = parse_matrix(&args.matrix, "matrix").map_err(invalid)?;
            let r = a.rank(1e-12);
            Ok(text_result(json!({ "rank": r }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct VecArgs {
    /// Vector or matrix as a flat list (vector) or rows (matrix).
    vector: Option<Vec<f64>>,
    matrix: Option<Vec<Vec<f64>>>,
    /// `"l2"` (default), `"l1"`, `"inf"`, or `"fro"` (matrix only).
    #[serde(default)]
    kind: Option<String>,
}

pub struct LinalgNorm;
impl Skill for LinalgNorm {
    fn name(&self) -> &'static str {
        "linalg_norm"
    }
    fn description(&self) -> &'static str {
        "Vector / matrix norm. Vector: `l1`, `l2` (default), `inf`. \
        Matrix: `fro` (Frobenius, default), `l1`, `inf`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<VecArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<VecArgs>()?;
            let kind = args.kind.unwrap_or_else(|| "l2".into()).to_lowercase();
            let n = match (&args.vector, &args.matrix) {
                (Some(v), None) => {
                    let v = parse_vector(v, "vector").map_err(invalid)?;
                    match kind.as_str() {
                        "l1" => v.iter().map(|x| x.abs()).sum::<f64>(),
                        "l2" => v.norm(),
                        "inf" => v.iter().fold(0_f64, |a, b| a.max(b.abs())),
                        other => return Err(invalid(format!("unknown vector norm '{other}'"))),
                    }
                }
                (None, Some(m)) => {
                    let m = parse_matrix(m, "matrix").map_err(invalid)?;
                    match kind.as_str() {
                        "fro" | "l2" => m.norm(),
                        "l1" => (0..m.ncols())
                            .map(|j| (0..m.nrows()).map(|i| m[(i, j)].abs()).sum::<f64>())
                            .fold(0_f64, f64::max),
                        "inf" => (0..m.nrows())
                            .map(|i| (0..m.ncols()).map(|j| m[(i, j)].abs()).sum::<f64>())
                            .fold(0_f64, f64::max),
                        other => return Err(invalid(format!("unknown matrix norm '{other}'"))),
                    }
                }
                _ => return Err(invalid("supply exactly one of `vector` / `matrix`")),
            };
            Ok(text_result(json!({ "norm": n }).to_string()))
        })
    }
}

pub struct LinalgMatmul;
impl Skill for LinalgMatmul {
    fn name(&self) -> &'static str {
        "linalg_matmul"
    }
    fn description(&self) -> &'static str {
        "Matrix-matrix product C = A·B. Errors if inner dimensions don't match."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MatMulArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<MatMulArgs>()?;
            let a = parse_matrix(&args.a, "a").map_err(invalid)?;
            let b = parse_matrix(&args.b, "b").map_err(invalid)?;
            if a.ncols() != b.nrows() {
                return Err(invalid(format!(
                    "inner dimensions don't match: A is {}×{}, B is {}×{}",
                    a.nrows(),
                    a.ncols(),
                    b.nrows(),
                    b.ncols()
                )));
            }
            let c = a * b;
            Ok(text_result(
                json!({ "result": matrix_to_rows(&c) }).to_string(),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(LinalgSolve),
        Box::new(LinalgLstsq),
        Box::new(LinalgSvd),
        Box::new(LinalgEigen),
        Box::new(LinalgQr),
        Box::new(LinalgInv),
        Box::new(LinalgDet),
        Box::new(LinalgRank),
        Box::new(LinalgNorm),
        Box::new(LinalgMatmul),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_2x2() {
        // [[1,2],[3,4]] x = [5,6]  → x = [-4, 4.5]
        let a = parse_matrix(&[vec![1.0, 2.0], vec![3.0, 4.0]], "a").unwrap();
        let b = parse_vector(&[5.0, 6.0], "b").unwrap();
        let x = a.lu().solve(&b).unwrap();
        assert!((x[0] - -4.0).abs() < 1e-9);
        assert!((x[1] - 4.5).abs() < 1e-9);
    }

    #[test]
    fn det_2x2() {
        let a = parse_matrix(&[vec![1.0, 2.0], vec![3.0, 4.0]], "a").unwrap();
        assert!((a.determinant() - (-2.0)).abs() < 1e-9);
    }

    #[test]
    fn ragged_rejected() {
        assert!(parse_matrix(&[vec![1.0, 2.0], vec![3.0]], "x").is_err());
    }
}
