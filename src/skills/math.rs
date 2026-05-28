//! Math skills (local, no network): `math_eval` evaluates an arithmetic/scientific
//! expression; `math_solve` solves a single-variable (in `x`) linear or quadratic
//! equation. Built on the `meval` expression evaluator (functions like sqrt, sin,
//! cos, tan, ln, log, exp, abs, floor, ceil; constants `pi`, `e`; `^` for power),
//! which covers arithmetic, geometry formulas, and evaluating algebraic expressions.

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use regex::Regex;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// Insert explicit `*` for the common implicit-multiplication cases meval can't
// parse: a number/`)` before the variable `x` (`2x` → `2*x`) or before a paren
// (`2(…)` → `2*(…)`). Safe around function names (`sin`, `max`, `exp`) and
// scientific notation, since those don't put a digit/`)` immediately before `x`/`(`.
static IMPLICIT_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9.)])x").unwrap());
static IMPLICIT_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9.)])\(").unwrap());

fn normalize(s: &str) -> String {
    let s = IMPLICIT_VAR.replace_all(s, "${1}*x");
    IMPLICIT_PAREN.replace_all(&s, "${1}*(").into_owned()
}

/// Tidy a float for display: damp float noise, then shortest round-trip form.
fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    let r = (x * 1e10).round() / 1e10;
    let r = if r == 0.0 { 0.0 } else { r }; // normalize -0.0
    format!("{r}")
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MathEvalArgs {
    /// The expression to evaluate, e.g. `2 + 3 * (4 - 1)`, `sqrt(2)`,
    /// `sin(pi/2)`, `3.14159 * 5^2` (area of a circle r=5).
    expression: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MathSolveArgs {
    /// A single-variable (in `x`) linear or quadratic equation, e.g.
    /// `2x + 3 = 7`, `x^2 - 5x + 6 = 0`. Without `=`, the expression is set to 0.
    equation: String,
}

pub struct MathEval;
impl Skill for MathEval {
    fn name(&self) -> &'static str {
        "math_eval"
    }
    fn description(&self) -> &'static str {
        "Evaluate a math expression (local, no network): arithmetic, functions (sqrt, sin, cos, \
        tan, ln, log, exp, abs, floor, ceil, …), constants (pi, e), and `^` for powers. Use for \
        arithmetic and for geometry/algebra formulas you write out, e.g. `pi*5^2`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MathEvalArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<MathEvalArgs>()?;
            let value = meval::eval_str(normalize(&args.expression))
                .map_err(|e| invalid(format!("could not evaluate '{}': {e}", args.expression)))?;
            Ok(text_result(format!(
                "{} = {}",
                args.expression.trim(),
                fmt_num(value)
            )))
        })
    }
}

pub struct MathSolve;
impl Skill for MathSolve {
    fn name(&self) -> &'static str {
        "math_solve"
    }
    fn description(&self) -> &'static str {
        "Solve a single-variable (in `x`) linear or quadratic equation, e.g. `2x + 3 = 7` or \
        `x^2 - 5x + 6 = 0`. Reports the root(s) (or notes complex roots / no solution). Local, no \
        network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MathSolveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<MathSolveArgs>()?;
            let out = solve(&args.equation).map_err(invalid)?;
            Ok(text_result(out))
        })
    }
}

/// Solve a linear/quadratic equation in `x`. Coefficients are recovered by
/// sampling f(x) = LHS − RHS at a few points (works regardless of how the
/// polynomial is written), then verified to be at most degree 2.
fn solve(equation: &str) -> Result<String, String> {
    let eq = equation.trim();
    let (lhs, rhs) = match eq.split_once('=') {
        Some((l, r)) => (l.trim(), r.trim()),
        None => (eq, "0"),
    };
    if lhs.is_empty() {
        return Err("the left-hand side is empty".into());
    }
    let src = normalize(&format!("({lhs})-({rhs})"));
    let expr: meval::Expr = src
        .parse()
        .map_err(|e| format!("could not parse equation: {e}"))?;
    let f = expr
        .bind("x")
        .map_err(|e| format!("the equation must be in a single variable `x`: {e}"))?;

    let f0 = f(0.0);
    let f1 = f(1.0);
    let fm1 = f(-1.0);
    let f2 = f(2.0);
    if [f0, f1, fm1, f2].iter().any(|v| !v.is_finite()) {
        return Err("the equation is undefined at the sampled points".into());
    }
    let c = f0;
    let b = (f1 - fm1) / 2.0;
    let a = (f1 + fm1) / 2.0 - f0;

    // Verify degree ≤ 2: a quadratic must reproduce f(2).
    let predicted = a * 4.0 + b * 2.0 + c;
    if (predicted - f2).abs() > 1e-6 * (1.0 + f2.abs()) {
        return Err("only linear and quadratic equations in `x` are supported".into());
    }

    if a.abs() < 1e-12 {
        // Linear: b*x + c = 0.
        if b.abs() < 1e-12 {
            return if c.abs() < 1e-9 {
                Ok("Any x is a solution (identity).".into())
            } else {
                Ok("No solution (contradiction).".into())
            };
        }
        let x = -c / b;
        return Ok(format!("Linear equation. x = {}", fmt_num(x)));
    }

    // Quadratic: a*x² + b*x + c = 0.
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        let re = -b / (2.0 * a);
        let im = (-disc).sqrt() / (2.0 * a).abs();
        return Ok(format!(
            "Quadratic (a={}, b={}, c={}). Complex roots: x = {} ± {}i",
            fmt_num(a),
            fmt_num(b),
            fmt_num(c),
            fmt_num(re),
            fmt_num(im)
        ));
    }
    let sq = disc.sqrt();
    let x1 = (-b + sq) / (2.0 * a);
    let x2 = (-b - sq) / (2.0 * a);
    if (x1 - x2).abs() < 1e-12 {
        Ok(format!(
            "Quadratic (a={}, b={}, c={}). Double root: x = {}",
            fmt_num(a),
            fmt_num(b),
            fmt_num(c),
            fmt_num(x1)
        ))
    } else {
        Ok(format!(
            "Quadratic (a={}, b={}, c={}). Roots: x = {} or x = {}",
            fmt_num(a),
            fmt_num(b),
            fmt_num(c),
            fmt_num(x1),
            fmt_num(x2)
        ))
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(MathEval), Box::new(MathSolve)]
}

#[cfg(test)]
mod tests {
    use super::solve;

    #[test]
    fn solves_linear() {
        assert!(solve("2x + 3 = 7").unwrap().contains("x = 2"));
    }

    #[test]
    fn solves_quadratic_two_roots() {
        let out = solve("x^2 - 5x + 6 = 0").unwrap();
        assert!(out.contains("x = 3") && out.contains("x = 2"));
    }

    #[test]
    fn rejects_higher_degree() {
        assert!(solve("x^3 = 8").is_err());
    }
}
