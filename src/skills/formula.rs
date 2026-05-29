//! Shared **formula-registry engine** used by the subject modules (`algebra`,
//! `geometry`, `trigonometry`, `physics`). It is not a skill itself — it provides the
//! [`Formula`] type and the `compute`/`list` helpers so each domain can expose its own
//! named-formula tools (`physics_formula`, `geometry_formula`, …) while the
//! lookup/validation/formatting logic lives in one place.
//!
//! A [`Formula`] maps required inputs → one output via a pure `eval` closure. Required
//! inputs are checked before `eval` runs, so `eval` can index `a["x"]` safely; optional
//! inputs are read with [`opt`]. Angles are in degrees by convention. SI units.

use std::collections::HashMap;

/// Variable values keyed by name.
pub(crate) type Args = HashMap<String, f64>;

/// One named variable with its unit ("" for dimensionless).
pub(crate) struct Var {
    pub name: &'static str,
    pub unit: &'static str,
}

/// Terse constructor for a [`Var`].
pub(crate) const fn v(name: &'static str, unit: &'static str) -> Var {
    Var { name, unit }
}

/// A named formula: required `inputs` → one `out`, computed by `eval`.
pub(crate) struct Formula {
    pub id: &'static str,
    pub category: &'static str,
    pub summary: &'static str,
    pub inputs: Vec<Var>,
    pub out: Var,
    pub eval: fn(&Args) -> f64,
}

/// Read an optional variable, falling back to `default`.
pub(crate) fn opt(a: &Args, k: &str, default: f64) -> f64 {
    a.get(k).copied().unwrap_or(default)
}

/// Permutations nPr = n·(n-1)·…·(n-r+1); NaN for invalid n/r.
pub(crate) fn npr(n: f64, r: f64) -> f64 {
    let (n, r) = (n.round(), r.round());
    if r < 0.0 || n < 0.0 || r > n {
        return f64::NAN;
    }
    let mut acc = 1.0;
    let mut i = 0.0;
    while i < r {
        acc *= n - i;
        i += 1.0;
    }
    acc
}

/// Factorial n! (as f64).
pub(crate) fn fact(n: f64) -> f64 {
    npr(n, n)
}

/// Compact number formatting: scientific for very large/small magnitudes, else a
/// trimmed fixed-point.
pub(crate) fn fmt_num(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    if !x.is_finite() {
        return "undefined".to_string();
    }
    let a = x.abs();
    if !(1e-4..1e7).contains(&a) {
        format!("{x:.6e}")
    } else {
        let s = format!("{x:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// One formula's signature: `in1(unit), in2(unit) → out(unit)`.
pub(crate) fn signature(f: &Formula) -> String {
    let label = |v: &Var| {
        if v.unit.is_empty() {
            v.name.to_string()
        } else {
            format!("{}({})", v.name, v.unit)
        }
    };
    let ins: Vec<String> = f.inputs.iter().map(label).collect();
    format!("{} → {}", ins.join(", "), label(&f.out))
}

/// Compute a named formula from `args`. Returns the formatted result, or an error
/// message (unknown id, missing inputs, or a non-finite/undefined result).
pub(crate) fn compute(formulas: &[Formula], name: &str, args: &Args) -> Result<String, String> {
    let id = name.trim();
    let f = formulas.iter().find(|f| f.id == id).ok_or_else(|| {
        format!("unknown formula '{id}' (use the *_formula_list tool to discover ids)")
    })?;
    let missing: Vec<&str> = f
        .inputs
        .iter()
        .filter(|v| !args.contains_key(v.name))
        .map(|v| v.name)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "{id} needs: {}. Missing: {}.",
            signature(f),
            missing.join(", ")
        ));
    }
    let result = (f.eval)(args);
    if !result.is_finite() {
        return Err(format!(
            "{id} is undefined for those inputs (division by zero or out-of-domain)."
        ));
    }
    let unit = if f.out.unit.is_empty() {
        String::new()
    } else {
        format!(" {}", f.out.unit)
    };
    Ok(format!(
        "{} = {}{}\n  {}",
        f.out.name,
        fmt_num(result),
        unit,
        f.summary
    ))
}

/// List formulas (id, equation, signature) grouped by category. `filter` matches a
/// category or any substring of the id/summary.
pub(crate) fn list(formulas: &[Formula], filter: Option<&str>) -> String {
    let filter = filter.map(|s| s.trim().to_ascii_lowercase());
    let mut rows: Vec<&Formula> = formulas
        .iter()
        .filter(|f| match &filter {
            None => true,
            Some(q) => {
                f.category.contains(q.as_str())
                    || f.id.contains(q.as_str())
                    || f.summary.to_ascii_lowercase().contains(q.as_str())
            }
        })
        .collect();
    if rows.is_empty() {
        return format!("No formulas match '{}'.", filter.unwrap_or_default());
    }
    rows.sort_by(|a, b| (a.category, a.id).cmp(&(b.category, b.id)));
    let mut out = format!("{} formula(s):", rows.len());
    let mut cat = "";
    for f in rows {
        if f.category != cat {
            cat = f.category;
            out.push_str(&format!("\n\n[{cat}]"));
        }
        out.push_str(&format!(
            "\n  {} — {}\n    {}",
            f.id,
            f.summary,
            signature(f)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_num_scientific_and_plain() {
        assert_eq!(fmt_num(9.0), "9");
        assert_eq!(fmt_num(2_598_960.0), "2598960");
        assert!(fmt_num(8.9e16).contains('e'));
    }

    #[test]
    fn npr_and_fact() {
        assert_eq!(npr(5.0, 2.0), 20.0);
        assert_eq!(fact(5.0), 120.0);
        assert!(npr(2.0, 5.0).is_nan());
    }

    #[test]
    fn compute_checks_missing_and_unknown() {
        let fs = vec![Formula {
            id: "double",
            category: "test",
            summary: "y = 2x",
            inputs: vec![v("x", "")],
            out: v("y", ""),
            eval: |a| 2.0 * a["x"],
        }];
        assert!(compute(&fs, "double", &Args::new()).is_err()); // missing x
        let mut a = Args::new();
        a.insert("x".into(), 3.0);
        assert!(compute(&fs, "double", &a).unwrap().contains("y = 6"));
        assert!(compute(&fs, "nope", &a).is_err());
    }
}
