//! Generic ODE integrator — RK4 with a programmable right-hand-side
//! evaluated via the `meval` expression engine. Pure math, on by default.
//!
//! The RHS is a list of expressions, one per state variable, each referring
//! to `t` (time) and `y0`, `y1`, … (current state). The integrator steps
//! a single fixed-step pass and returns the full trajectory.
//!
//! Tools: `ode_rk4` (4th-order classical Runge-Kutta).

use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use meval::{Context, Expr};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Rk4Args {
    /// One expression per state variable. Each may reference `t` (current
    /// time) and `y0`, `y1`, … (current state).
    /// Example for projectile motion (x, vx, y, vy under gravity g=-9.81):
    ///   [`y1`, `0`, `y3`, `-9.81`]
    rhs: Vec<String>,
    /// Initial state values, same length as `rhs`.
    y0: Vec<f64>,
    /// Start time.
    t_start: f64,
    /// End time (must be > `t_start`).
    t_end: f64,
    /// Number of integration steps (default 100, max 10000).
    #[serde(default)]
    steps: Option<usize>,
}

/// Compile each RHS expression once into a `meval::Expr` for repeated
/// evaluation inside the integrator's inner loop.
fn compile_rhs(rhs: &[String]) -> Result<Vec<Expr>> {
    rhs.iter()
        .map(|s| {
            s.parse::<Expr>()
                .map_err(|e| anyhow!("rhs parse error: {e}"))
        })
        .collect()
}

/// Evaluate every RHS at the given (t, y) state.
fn eval_rhs(exprs: &[Expr], t: f64, y: &[f64]) -> Result<Vec<f64>> {
    let mut ctx = Context::new();
    ctx.var("t", t);
    for (i, v) in y.iter().enumerate() {
        ctx.var(format!("y{i}"), *v);
    }
    exprs
        .iter()
        .map(|e| {
            e.eval_with_context(&ctx)
                .map_err(|e| anyhow!("rhs eval error: {e}"))
        })
        .collect()
}

pub struct OdeRk4;
impl Skill for OdeRk4 {
    fn name(&self) -> &'static str {
        "ode_rk4"
    }
    fn description(&self) -> &'static str {
        "Solve a system of ODEs via the classical fourth-order Runge-Kutta \
        method. The RHS is supplied as a list of expressions referring to \
        `t` and `y0`, `y1`, … — one per state variable. Returns the \
        trajectory as parallel arrays of time and state values. Bounded at \
        10 000 steps."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<Rk4Args>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, args) = ctx.parse::<Rk4Args>()?;
            if args.rhs.len() != args.y0.len() {
                return Err(invalid("rhs and y0 must have the same length"));
            }
            if args.t_end <= args.t_start {
                return Err(invalid("t_end must be > t_start"));
            }
            let n_steps = args.steps.unwrap_or(100).clamp(1, 10_000);
            let h = (args.t_end - args.t_start) / n_steps as f64;
            let exprs = compile_rhs(&args.rhs).map_err(invalid)?;

            let mut t = args.t_start;
            let mut y: Vec<f64> = args.y0.clone();
            let mut t_out = Vec::with_capacity(n_steps + 1);
            let mut y_out: Vec<Vec<f64>> = (0..y.len())
                .map(|_| Vec::with_capacity(n_steps + 1))
                .collect();
            t_out.push(t);
            for (col, v) in y.iter().enumerate() {
                y_out[col].push(*v);
            }

            for _ in 0..n_steps {
                let k1 = eval_rhs(&exprs, t, &y).map_err(invalid)?;
                let y_mid: Vec<f64> = y
                    .iter()
                    .zip(&k1)
                    .map(|(yi, ki)| yi + 0.5 * h * ki)
                    .collect();
                let k2 = eval_rhs(&exprs, t + 0.5 * h, &y_mid).map_err(invalid)?;
                let y_mid2: Vec<f64> = y
                    .iter()
                    .zip(&k2)
                    .map(|(yi, ki)| yi + 0.5 * h * ki)
                    .collect();
                let k3 = eval_rhs(&exprs, t + 0.5 * h, &y_mid2).map_err(invalid)?;
                let y_end: Vec<f64> = y.iter().zip(&k3).map(|(yi, ki)| yi + h * ki).collect();
                let k4 = eval_rhs(&exprs, t + h, &y_end).map_err(invalid)?;
                for i in 0..y.len() {
                    y[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
                }
                t += h;
                t_out.push(t);
                for (col, v) in y.iter().enumerate() {
                    y_out[col].push(*v);
                }
            }

            Ok(text_result(
                json!({
                    "t": t_out,
                    "y": y_out,
                })
                .to_string(),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(OdeRk4)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_decay() {
        // dy/dt = -y, y(0)=1, t=[0,1] → y(1) = exp(-1) ≈ 0.367879
        let exprs = compile_rhs(&["-y0".into()]).unwrap();
        let mut t = 0.0;
        let mut y = vec![1.0_f64];
        let h = 0.01;
        for _ in 0..100 {
            let k1 = eval_rhs(&exprs, t, &y).unwrap();
            let y_mid: Vec<f64> = y
                .iter()
                .zip(&k1)
                .map(|(yi, ki)| yi + 0.5 * h * ki)
                .collect();
            let k2 = eval_rhs(&exprs, t + 0.5 * h, &y_mid).unwrap();
            let y_mid2: Vec<f64> = y
                .iter()
                .zip(&k2)
                .map(|(yi, ki)| yi + 0.5 * h * ki)
                .collect();
            let k3 = eval_rhs(&exprs, t + 0.5 * h, &y_mid2).unwrap();
            let y_end: Vec<f64> = y.iter().zip(&k3).map(|(yi, ki)| yi + h * ki).collect();
            let k4 = eval_rhs(&exprs, t + h, &y_end).unwrap();
            for i in 0..y.len() {
                y[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
            }
            t += h;
        }
        assert!((y[0] - (-1.0_f64).exp()).abs() < 1e-6);
    }
}
