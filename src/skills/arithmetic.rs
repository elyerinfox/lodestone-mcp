//! Arithmetic skill (local, no network): `arithmetic_eval` evaluates a free-form
//! arithmetic/scientific expression via the `meval` evaluator (sqrt, sin, cos, tan,
//! ln, log, exp, abs, floor, ceil; constants `pi`, `e`; `^` for powers). Also hosts
//! the small display/normalization helpers (`fmt_num`, `normalize`) reused by the
//! other math-field modules (`algebra`, `geometry`, `physics`).

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use regex::Regex;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// Insert explicit `*` for the common implicit-multiplication cases meval can't
// parse: a number/`)` before the variable `x` (`2x` → `2*x`) or before a paren.
static IMPLICIT_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9.)])x").unwrap());
static IMPLICIT_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9.)])\(").unwrap());

/// Normalize a meval-bound expression: rewrite Python-style `**` to `^`
/// (LLMs reach for `**` half the time — `meval` only understands `^`), then
/// insert explicit `*` so `2x` / `2(…)` parse. Shared with `algebra`.
pub(crate) fn normalize(s: &str) -> String {
    let s = s.replace("**", "^");
    let s = IMPLICIT_VAR.replace_all(&s, "${1}*x");
    IMPLICIT_PAREN.replace_all(&s, "${1}*(").into_owned()
}

/// Tidy a float for display: damp float noise, then shortest round-trip form.
/// Shared by the math-field modules' tool output.
pub(crate) fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    let r = (x * 1e10).round() / 1e10;
    let r = if r == 0.0 { 0.0 } else { r }; // normalize -0.0
    format!("{r}")
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvalArgs {
    /// The expression to evaluate, e.g. `2 + 3 * (4 - 1)`, `sqrt(2)`,
    /// `sin(pi/2)`, `3.14159 * 5^2`.
    expression: String,
}

pub struct ArithmeticEval;
impl Skill for ArithmeticEval {
    fn name(&self) -> &'static str {
        "arithmetic_eval"
    }
    fn description(&self) -> &'static str {
        "Evaluate a math expression (local, no network): arithmetic, functions (sqrt, sin, cos, \
        tan, ln, log, exp, abs, floor, ceil, …), constants (pi, e), and `^` for powers. Use for \
        free-form arithmetic and expressions you write out, e.g. `pi*5^2`. (For named formulas use \
        the *_formula tools; for equations use algebra_solve.)"
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EvalArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<EvalArgs>()?;
            let value = meval::eval_str(normalize(&args.expression))
                .map_err(|e| invalid(format!("could not evaluate '{}': {e}", args.expression)))?;
            Ok(text_result(format!(
                "{} = {}",
                args.expression.trim(),
                fmt_num(value)
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Order-of-operations arithmetic",
                args: r#"{"expression": "2 + 3 * (4 - 1)"}"#,
                note: Some("Returns `2 + 3 * (4 - 1) = 11`."),
            },
            SkillExample {
                title: "Functions and constants",
                args: r#"{"expression": "pi * 5^2"}"#,
                note: Some("`pi` and `e` are recognized; `^` is the power operator."),
            },
            SkillExample {
                title: "Python-style `**` accepted",
                args: r#"{"expression": "2**10"}"#,
                note: Some("`**` is rewritten to `^` before evaluation."),
            },
            SkillExample {
                title: "Trig in radians",
                args: r#"{"expression": "sin(pi/2)"}"#,
                note: Some("Trig args are radians here; for degrees use `trig_formula`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Evaluate a free-form arithmetic expression with parens and functions.",
            "Plug numbers into a one-shot formula you've already written out.",
            "Quick sanity-check of a hand calculation without naming a formula.",
        ]
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(ArithmeticEval)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_implicit_multiplication() {
        assert_eq!(normalize("2x + 3"), "2*x + 3");
        assert_eq!(normalize("2(3+4)"), "2*(3+4)");
    }

    #[test]
    fn normalizes_python_power_operator() {
        assert_eq!(normalize("2**10"), "2^10");
        assert_eq!(meval::eval_str(normalize("2**10")).unwrap(), 1024.0);
        // Combines with implicit-multiplication insertion.
        assert_eq!(normalize("2**(3+1)"), "2^(3+1)");
    }

    #[test]
    fn fmt_num_trims_noise() {
        assert_eq!(fmt_num(0.1 + 0.2), "0.3");
        assert_eq!(fmt_num(2.0), "2");
    }

    #[test]
    fn eval_basic() {
        assert_eq!(meval::eval_str(normalize("2+3*(4-1)")).unwrap(), 11.0);
    }
}
