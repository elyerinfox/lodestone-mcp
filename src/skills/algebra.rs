//! Algebra skill (local, no network): `algebra_solve` solves a single-variable (in
//! `x`) linear or quadratic equation; `algebra_formula` / `algebra_formula_list`
//! cover named algebra/combinatorics formulas (permutations, combinations).

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::arithmetic::{fmt_num, normalize};
use crate::skills::formula::{self, v, Args, Formula};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[rustfmt::skip]
static FORMULAS: LazyLock<Vec<Formula>> = LazyLock::new(|| {
    vec![
        Formula { id: "permutations", category: "combinatorics", summary: "Permutations: nPr = n!/(n-r)!", inputs: vec![v("n",""), v("r","")], out: v("nPr",""), eval: |a| formula::npr(a["n"], a["r"]) },
        Formula { id: "combinations", category: "combinatorics", summary: "Combinations: nCr = n!/(r!·(n-r)!)", inputs: vec![v("n",""), v("r","")], out: v("nCr",""), eval: |a| formula::npr(a["n"], a["r"]) / formula::fact(a["r"]) },
        Formula { id: "factorial", category: "combinatorics", summary: "Factorial: n!", inputs: vec![v("n","")], out: v("n!",""), eval: |a| formula::fact(a["n"]) },
        Formula { id: "quadratic_discriminant", category: "algebra", summary: "Discriminant: Δ = b² - 4·a·c", inputs: vec![v("a",""), v("b",""), v("c","")], out: v("discriminant",""), eval: |a| a["b"].powi(2) - 4.0*a["a"]*a["c"] },
    ]
});

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolveArgs {
    /// A single-variable (in `x`) linear or quadratic equation, e.g.
    /// `2x + 3 = 7`, `x^2 - 5x + 6 = 0`. Without `=`, the expression is set to 0.
    equation: String,
}

pub struct AlgebraSolve;
impl Skill for AlgebraSolve {
    fn name(&self) -> &'static str {
        "algebra_solve"
    }
    fn description(&self) -> &'static str {
        "Solve a single-variable (in `x`) linear or quadratic equation, e.g. `2x + 3 = 7` or \
        `x^2 - 5x + 6 = 0`. Reports the root(s) (or notes complex roots / no solution). Local, no \
        network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<SolveArgs>()?;
            let out = solve(&args.equation).map_err(invalid)?;
            Ok(text_result(out))
        })
    }
}

/// Solve a linear/quadratic equation in `x`. Coefficients are recovered by sampling
/// f(x) = LHS − RHS at a few points, then verified to be at most degree 2.
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormulaArgs {
    /// Formula id (see `algebra_formula_list`), e.g. `combinations`.
    name: String,
    /// Variable values, e.g. `{"n": 52, "r": 5}`.
    #[serde(default)]
    args: Args,
}

pub struct AlgebraFormula;
impl Skill for AlgebraFormula {
    fn name(&self) -> &'static str {
        "algebra_formula"
    }
    fn description(&self) -> &'static str {
        "Compute a named algebra/combinatorics formula (permutations, combinations, factorial, \
        quadratic discriminant). Pass `name` (see algebra_formula_list) and `args` as a {var: \
        value} map. For solving equations use algebra_solve."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FormulaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<FormulaArgs>()?;
            let out = formula::compute(&FORMULAS, &args.name, &args.args).map_err(invalid)?;
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListArgs {
    /// Optional id/equation substring filter.
    #[serde(default)]
    filter: Option<String>,
}

pub struct AlgebraFormulaList;
impl Skill for AlgebraFormulaList {
    fn name(&self) -> &'static str {
        "algebra_formula_list"
    }
    fn description(&self) -> &'static str {
        "List the named algebra/combinatorics formulas (id, equation, signature). Feed an id to \
        algebra_formula."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ListArgs>()?;
            Ok(text_result(formula::list(
                &FORMULAS,
                args.filter.as_deref(),
            )))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(AlgebraSolve),
        Box::new(AlgebraFormula),
        Box::new(AlgebraFormulaList),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, args: &[(&str, f64)]) -> f64 {
        let f = FORMULAS.iter().find(|f| f.id == id).unwrap();
        let map: Args = args.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        (f.eval)(&map)
    }

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

    #[test]
    fn combinatorics() {
        assert_eq!(run("combinations", &[("n", 52.0), ("r", 5.0)]), 2_598_960.0);
        assert_eq!(run("permutations", &[("n", 5.0), ("r", 2.0)]), 20.0);
        assert_eq!(run("factorial", &[("n", 5.0)]), 120.0);
        assert_eq!(
            run(
                "quadratic_discriminant",
                &[("a", 1.0), ("b", -5.0), ("c", 6.0)]
            ),
            1.0
        );
    }
}
