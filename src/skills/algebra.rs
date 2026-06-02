//! Algebra skill (local, no network): `algebra_solve` solves a single-variable
//! linear or quadratic equation (the variable doesn't have to be `x`);
//! `algebra_formula` / `algebra_formula_list` cover named algebra/combinatorics
//! formulas (permutations, combinations).
//!
//! `algebra_solve` is generous with what LLMs hand it: trailing prose like
//! `, find x` or `-> solve for t (velocity needed for 5,000 km range)` is
//! stripped, Python-style `**` is rewritten to `^`, and a `where var=val, ...`
//! clause is parsed and substituted before the solve so equations like
//! `s = u*t + 0.5*a*t^2 where s=1000, u=800, a=-9.81 -> solve for t`
//! land cleanly.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use regex::Regex;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::arithmetic::fmt_num;
use crate::skills::formula::{self, v, Args, Formula};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// Trailing prose markers an LLM tends to append: `-> solve for X`, `, find X`,
// `solve for x`, anything after them is description. Greedy `.*` to the end.
static TRAILING_PROSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*(->|,)?\s*(solve|find)\b.*$").unwrap());

// `where var=val, var=val, ...` substitution clause. Captures everything after
// `where` to the end of the (already-trailer-stripped) string.
static WHERE_CLAUSE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*\bwhere\b\s*(.*)$").unwrap());

// Numeric literal (including scientific notation) — stripped before identifier
// detection so `2e3` doesn't get mistaken for a variable named `e3`.
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?").unwrap());

// Identifier (variable or function name) in an expression.
static IDENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-zA-Z][a-zA-Z0-9_]*").unwrap());

// Names meval recognizes as builtins (functions + constants); never treated as
// free variables to solve for.
const MEVAL_BUILTINS: &[&str] = &[
    "sqrt", "abs", "exp", "ln", "log", "log2", "log10", "sin", "cos", "tan", "asin", "acos",
    "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh", "atanh", "floor", "ceil", "round",
    "signum", "max", "min", "pi", "e",
];

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
        "Solve a single-variable linear or quadratic equation (the variable can be any letter — \
        `x`, `t`, `v`, …), e.g. `2x + 3 = 7`, `x^2 - 5x + 6 = 0`, `800*t - 4.905*t^2 = 1000`. \
        Reports the root(s) (or notes complex roots / no solution). Accepts trailing prose like \
        `, find x` or `-> solve for t (description)`; Python-style `**` is accepted; a \
        `where var=val, var=val, ...` clause substitutes named parameters before solving \
        (e.g. `s = u*t + 0.5*a*t^2 where s=1000, u=800, a=-9.81 -> solve for t`). Local, no \
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Simple linear",
                args: r#"{"equation": "2x + 3 = 7"}"#,
                note: Some("Returns `Linear equation. x = 2`."),
            },
            SkillExample {
                title: "Quadratic, two real roots",
                args: r#"{"equation": "x^2 - 5x + 6 = 0"}"#,
                note: Some("Returns `Quadratic (...). Roots: x = 3 or x = 2`."),
            },
            SkillExample {
                title: "Any variable name, not just x",
                args: r#"{"equation": "800*t - 4.905*t^2 = 1000"}"#,
                note: Some("Auto-detects `t` as the unknown."),
            },
            SkillExample {
                title: "Named-parameter substitution + trailing prose",
                args: r#"{"equation": "s = u*t + 0.5*a*t^2 where s=1000, u=800, a=-9.81 -> solve for t"}"#,
                note: Some("Strips the `-> solve for t` trailer and substitutes the `where` values before solving."),
            },
            SkillExample {
                title: "Python-style `**` power operator",
                args: r#"{"equation": "x**2 = 49"}"#,
                note: Some("`**` is rewritten to `^` before parsing."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Solve a single-variable linear or quadratic equation symbolically.",
            "Invert a closed-form physics / projectile / finance formula with one unknown.",
            "Verify an answer the LLM derived by hand against a parsed solver.",
        ]
    }
}

/// Parse a `var=value, var=value, ...` substitution clause.
fn parse_substitutions(s: &str) -> Result<HashMap<String, f64>, String> {
    let mut subs = HashMap::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, val) = part
            .split_once('=')
            .ok_or_else(|| format!("expected `var=value` pair in `where`, got `{part}`"))?;
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(format!(
                "missing variable name in `where` clause near `{part}`"
            ));
        }
        let val: f64 = val
            .trim()
            .parse()
            .map_err(|_| format!("could not parse `{}` as a number in `{part}`", val.trim()))?;
        subs.insert(name, val);
    }
    Ok(subs)
}

/// Substitute named variables with their literal values, wrapped in parens so a
/// negative substitution can't flip operator precedence at the call site.
fn substitute(expr: &str, subs: &HashMap<String, f64>) -> String {
    if subs.is_empty() {
        return expr.to_string();
    }
    let mut out = String::with_capacity(expr.len());
    let mut last = 0;
    for m in IDENT.find_iter(expr) {
        out.push_str(&expr[last..m.start()]);
        if let Some(v) = subs.get(m.as_str()) {
            out.push_str(&format!("({v})"));
        } else {
            out.push_str(m.as_str());
        }
        last = m.end();
    }
    out.push_str(&expr[last..]);
    out
}

/// Identifiers that appear in an expression, ignoring meval builtins and any
/// names that have already been substituted away. Numbers (incl. scientific
/// notation) are stripped first so `2e3` doesn't get mis-read as a variable.
fn free_variables(expr: &str, subs: &HashMap<String, f64>) -> BTreeSet<String> {
    let stripped = NUMBER.replace_all(expr, " ");
    let mut out = BTreeSet::new();
    for m in IDENT.find_iter(&stripped) {
        let name = m.as_str();
        if MEVAL_BUILTINS.contains(&name) || subs.contains_key(name) {
            continue;
        }
        out.insert(name.to_string());
    }
    out
}

/// Normalize an expression for meval after the unknown is known: rewrite `**`
/// to `^`, insert explicit `*` where a number/`)` butts up against the unknown
/// or an opening paren.
fn normalize_for(expr: &str, var: &str) -> String {
    let s = expr.replace("**", "^");
    let v = regex::escape(var);
    // `2t` -> `2*t`, `)t` -> `)*t`.
    let implicit_var = Regex::new(&format!(r"([0-9.)])({v})")).unwrap();
    let s = implicit_var.replace_all(&s, "${1}*${2}").into_owned();
    // `2(…)` -> `2*(…)`, `)(…)` -> `)*(…)`.
    let implicit_paren = Regex::new(r"([0-9.)])\(").unwrap();
    implicit_paren.replace_all(&s, "${1}*(").into_owned()
}

/// Solve a linear/quadratic equation in a single auto-detected variable. The
/// equation may carry trailing prose (`, find x`, `-> solve for t (...)`) and
/// a `where var=val, ...` clause; both are stripped/substituted before solving.
fn solve(equation: &str) -> Result<String, String> {
    let mut eq = equation.trim().to_string();

    // 1. Strip trailing prose ("-> solve for x", ", find t", "solve for v…").
    if let Some(m) = TRAILING_PROSE.find(&eq) {
        eq = eq[..m.start()].trim_end().to_string();
    }

    // 2. Pull out a `where var=val, ...` clause and parse substitutions.
    let subs = if let Some(cap) = WHERE_CLAUSE.captures(&eq) {
        let whole = cap.get(0).unwrap();
        let parsed = parse_substitutions(&cap[1])?;
        eq = eq[..whole.start()].trim_end().to_string();
        parsed
    } else {
        HashMap::new()
    };

    if eq.is_empty() {
        return Err("empty equation".into());
    }

    // 3. Split on `=` (or treat the whole thing as `expr = 0`), substitute the
    //    `where`-bound names, and build f(x) = LHS − RHS.
    let (lhs, rhs) = match eq.split_once('=') {
        Some((l, r)) => (l.trim().to_string(), r.trim().to_string()),
        None => (eq, "0".to_string()),
    };
    if lhs.is_empty() {
        return Err("the left-hand side is empty".into());
    }
    let lhs = substitute(&lhs, &subs);
    let rhs = substitute(&rhs, &subs);
    let composite = format!("({lhs})-({rhs})");

    // 4. Identify the single free variable.
    let frees = free_variables(&composite, &subs);
    let var = match frees.len() {
        0 => {
            return Err(
                "no free variable to solve for. (For pure arithmetic, use arithmetic_eval.)".into(),
            );
        }
        1 => frees.into_iter().next().unwrap(),
        _ => {
            let list: Vec<String> = frees.into_iter().collect();
            return Err(format!(
                "the equation has multiple free variables ({}). Supply values for all but one via `where var=value, ...`.",
                list.join(", ")
            ));
        }
    };

    // 5. Normalize implicit multiplication around THIS variable and parse.
    let src = normalize_for(&composite, &var);
    let expr: meval::Expr = src.parse().map_err(|e| {
        format!("could not parse equation after substitution: {e} (source: `{src}`)")
    })?;
    let f = expr
        .bind(&var)
        .map_err(|e| format!("could not bind solve variable `{var}`: {e}"))?;

    // 6. Recover coefficients by sampling f at four points.
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

    let predicted = a * 4.0 + b * 2.0 + c;
    if (predicted - f2).abs() > 1e-6 * (1.0 + f2.abs()) {
        return Err("only linear and quadratic equations are supported".into());
    }

    if a.abs() < 1e-12 {
        if b.abs() < 1e-12 {
            return if c.abs() < 1e-9 {
                Ok(format!("Any {var} is a solution (identity)."))
            } else {
                Ok("No solution (contradiction).".into())
            };
        }
        let x = -c / b;
        return Ok(format!("Linear equation. {var} = {}", fmt_num(x)));
    }

    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        let re = -b / (2.0 * a);
        let im = (-disc).sqrt() / (2.0 * a).abs();
        return Ok(format!(
            "Quadratic (a={}, b={}, c={}). Complex roots: {var} = {} ± {}i",
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
            "Quadratic (a={}, b={}, c={}). Double root: {var} = {}",
            fmt_num(a),
            fmt_num(b),
            fmt_num(c),
            fmt_num(x1)
        ))
    } else {
        Ok(format!(
            "Quadratic (a={}, b={}, c={}). Roots: {var} = {} or {var} = {}",
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
    fn solves_non_x_variable() {
        // 800·t - 4.905·t² = 1000 → projectile-style quadratic.
        let out = solve("800*t - 4.905*t^2 = 1000").unwrap();
        assert!(out.contains("t = "), "got: {out}");
        // Sanity: both roots are positive (we're solving t when 0 < t and the
        // parabola opens downward), so the smaller root is ~1.27 and the
        // larger ~161.9 — both should appear.
        assert!(out.contains("Quadratic"));
    }

    #[test]
    fn strips_trailing_solve_for_prose() {
        let out =
            solve("x^2 / 9.81 = 5000 -> solve for x (velocity needed for 5,000 km range)").unwrap();
        // x² / 9.81 = 5000 → x² = 49050 → x ≈ ±221.47.
        assert!(
            out.contains("x = 221") || out.contains("x = -221"),
            "got: {out}"
        );
    }

    #[test]
    fn strips_trailing_find_x_prose() {
        let out = solve("x^2 / 9.81 = 5000000, find x").unwrap();
        // x² = 49 050 000 → x ≈ ±7 003.57.
        assert!(out.contains("7003") || out.contains("-7003"), "got: {out}");
    }

    #[test]
    fn handles_where_substitution() {
        // s = u·t + 0.5·a·t² with s=1000, u=800, a=-9.81 → 1000 = 800t - 4.905t².
        let out = solve("s = u*t + 0.5*a*t^2 where s=1000, u=800, a=-9.81 -> solve for t").unwrap();
        assert!(out.contains("t = "), "got: {out}");
        assert!(out.contains("Quadratic"));
    }

    #[test]
    fn accepts_python_power_operator() {
        // 2**x style — `**` rewritten to `^`.
        assert!(solve("x**2 = 49").unwrap().contains("x = 7"));
    }

    #[test]
    fn multiple_free_variables_is_an_error() {
        let err = solve("a*x + b = 0").unwrap_err();
        assert!(err.contains("multiple free variables"), "got: {err}");
    }

    #[test]
    fn no_free_variable_is_an_error() {
        let err = solve("2 + 3 = 5").unwrap_err();
        assert!(err.contains("no free variable") || err.contains("arithmetic_eval"));
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
