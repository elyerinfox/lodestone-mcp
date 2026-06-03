//! Structured validation framework for the `Skill` contract.
//!
//! Every skill declares a list of [`Rule`]s in its
//! [`Skill::validation_rules`](crate::skills::Skill::validation_rules)
//! implementation. The dispatcher evaluates them after `ctx.parse()`
//! succeeds and BEFORE the call body runs; on failure the call returns
//! a structured `{"validation_failed": [...]}` payload describing
//! exactly which fields broke which rules, so the LLM can correct
//! itself without parsing English error strings.
//!
//! The same rule tree is surfaced through `describe_skill` so the LLM
//! can audit the constraints up-front. It complements the JSON Schema
//! that comes from `schemars` derives — JSON Schema tells the LLM the
//! shape (types, required fields), `validation_rules` tells it the
//! domain constraints (range, mutual exclusion, allowed enum values,
//! regex shape).
//!
//! ## Composability
//!
//! Rules nest with `All` (AND), `Any` (OR), and `Not`. `ExactlyOne` and
//! `AtLeastOne` over a set of field names express the common
//! mutually-exclusive / "supply one of" patterns natively, so skills
//! don't have to roll their own. The `Custom` variant is the escape
//! hatch for anything the declarative DSL can't express.

use rmcp::model::JsonObject;
use serde_json::{json, Value};

/// One field-level constraint violation, structured for the LLM to read.
#[derive(Debug, Clone)]
pub struct FieldViolation {
    /// JSON-pointer-ish field path. Top-level field names ("`code`"), nested
    /// dotted paths ("`config.timeout`"), or array elements ("`items[2]`").
    pub field: String,
    /// Short rule identifier — `"range"`, `"one_of"`, `"regex"`, `"length"`,
    /// `"exactly_one"`, `"at_least_one"`, `"all_of"`, `"any_of"`, `"not"`,
    /// `"custom"`.
    pub rule: &'static str,
    /// Human-readable description of what's wrong.
    pub message: String,
    /// Machine-readable description of the expected shape, e.g.
    /// `{"min":100, "max":599}` or `{"one_of":["a","b"]}` — exactly what
    /// the rule asserts.
    pub expected: Value,
    /// The actual value that violated, when extractable.
    pub got: Option<Value>,
}

/// Outcome of [`Skill::validate`](crate::skills::Skill::validate).
#[derive(Debug, Clone)]
pub enum ValidationResult {
    /// Args passed every rule.
    Pass,
    /// One or more rules failed. The list is preserved in declaration
    /// order so the dispatcher can surface them deterministically.
    Fail(Vec<FieldViolation>),
}

impl ValidationResult {
    #[allow(dead_code)] // public API surface — referenced by tests + downstream callers.
    pub fn is_pass(&self) -> bool {
        matches!(self, ValidationResult::Pass)
    }
    /// Render as the structured JSON the dispatcher returns to the LLM.
    pub fn to_payload(&self) -> Value {
        match self {
            ValidationResult::Pass => json!({"validation": "pass"}),
            ValidationResult::Fail(violations) => {
                let arr: Vec<Value> = violations
                    .iter()
                    .map(|v| {
                        let mut obj = json!({
                            "field": v.field,
                            "rule": v.rule,
                            "message": v.message,
                            "expected": v.expected,
                        });
                        if let Some(g) = &v.got {
                            obj["got"] = g.clone();
                        }
                        obj
                    })
                    .collect();
                json!({"validation_failed": arr})
            }
        }
    }
}

/// Declarative validation rule. Built once per skill (typically as a
/// `&'static [Rule]`) so there's no per-call allocation cost.
///
/// Several variants (Regex, AtLeastOne, All, Not, Custom) aren't yet used by
/// any in-tree skill — they're part of the DSL surface so contributors can
/// reach for them without coordinating a trait change first.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum Rule {
    /// Numeric `field` must satisfy `min <= value <= max`. Either bound is
    /// optional. Works for any field that parses as `f64`.
    Range {
        field: &'static str,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// String `field` must equal one of the listed values (case-sensitive).
    OneOf {
        field: &'static str,
        values: &'static [&'static str],
    },
    /// String `field` must match the given Rust regex. `summary` is a
    /// short human description ("ISO-3166 alpha-2") shown in the error.
    Regex {
        field: &'static str,
        pattern: &'static str,
        summary: &'static str,
    },
    /// String / array `field` length bounds (Unicode chars for strings, len for arrays).
    Length {
        field: &'static str,
        min: Option<usize>,
        max: Option<usize>,
    },
    /// Exactly one of the named fields must be present + non-null.
    ExactlyOne {
        fields: &'static [&'static str],
    },
    /// At least one of the named fields must be present + non-null.
    AtLeastOne {
        fields: &'static [&'static str],
    },
    /// Conjunction — every sub-rule must pass.
    All(&'static [Rule]),
    /// Disjunction — at least one sub-rule must pass. Failures are reported
    /// only when EVERY branch fails (aggregated).
    Any(&'static [Rule]),
    /// Negation — the inner rule must NOT match. Useful as the dual of `OneOf`.
    Not(&'static Rule),
    /// Custom validator. Receives the full args object, returns Ok or one
    /// FieldViolation. Use sparingly — declarative variants are preferred
    /// because `describe_skill` can render them.
    Custom {
        /// Stable identifier shown in error output and `describe_skill`.
        name: &'static str,
        /// One-line summary shown in `describe_skill`.
        summary: &'static str,
        eval: fn(&JsonObject) -> Result<(), FieldViolation>,
    },
}

/// Evaluate every rule against the parsed arg object.
pub fn evaluate(rules: &[Rule], args: &JsonObject) -> ValidationResult {
    let mut out: Vec<FieldViolation> = Vec::new();
    for r in rules {
        if let Err(mut v) = eval_one(r, args) {
            out.append(&mut v);
        }
    }
    if out.is_empty() {
        ValidationResult::Pass
    } else {
        ValidationResult::Fail(out)
    }
}

fn eval_one(rule: &Rule, args: &JsonObject) -> Result<(), Vec<FieldViolation>> {
    match rule {
        Rule::Range { field, min, max } => {
            let v = lookup(args, field);
            // None / null is treated as "absent" — skip; let required-field shape catch it.
            let Some(value) = v else { return Ok(()) };
            let n = match value.as_f64() {
                Some(n) => n,
                None => {
                    return Err(vec![FieldViolation {
                        field: (*field).to_string(),
                        rule: "range",
                        message: format!("`{field}` must be a number"),
                        expected: json!({"type": "number"}),
                        got: Some(value.clone()),
                    }]);
                }
            };
            let lo = min.unwrap_or(f64::NEG_INFINITY);
            let hi = max.unwrap_or(f64::INFINITY);
            if n < lo || n > hi {
                return Err(vec![FieldViolation {
                    field: (*field).to_string(),
                    rule: "range",
                    message: format!(
                        "`{field}` must be in [{}..{}], got {n}",
                        min.map(|x| x.to_string()).unwrap_or_else(|| "-∞".into()),
                        max.map(|x| x.to_string()).unwrap_or_else(|| "+∞".into()),
                    ),
                    expected: json!({"min": min, "max": max}),
                    got: Some(json!(n)),
                }]);
            }
            Ok(())
        }
        Rule::OneOf { field, values } => {
            let v = lookup(args, field);
            let Some(value) = v else { return Ok(()) };
            let s = match value.as_str() {
                Some(s) => s,
                None => {
                    return Err(vec![FieldViolation {
                        field: (*field).to_string(),
                        rule: "one_of",
                        message: format!("`{field}` must be a string"),
                        expected: json!({"one_of": values}),
                        got: Some(value.clone()),
                    }]);
                }
            };
            if values.contains(&s) {
                Ok(())
            } else {
                Err(vec![FieldViolation {
                    field: (*field).to_string(),
                    rule: "one_of",
                    message: format!(
                        "`{field}` must be one of {values:?}, got `{s}`"
                    ),
                    expected: json!({"one_of": values}),
                    got: Some(json!(s)),
                }])
            }
        }
        Rule::Regex { field, pattern, summary } => {
            let v = lookup(args, field);
            let Some(value) = v else { return Ok(()) };
            let s = match value.as_str() {
                Some(s) => s,
                None => {
                    return Err(vec![FieldViolation {
                        field: (*field).to_string(),
                        rule: "regex",
                        message: format!("`{field}` must be a string"),
                        expected: json!({"pattern": pattern, "summary": summary}),
                        got: Some(value.clone()),
                    }]);
                }
            };
            // Compile per-call; regex caching across calls is a follow-up.
            match regex::Regex::new(pattern) {
                Ok(re) if re.is_match(s) => Ok(()),
                Ok(_) => Err(vec![FieldViolation {
                    field: (*field).to_string(),
                    rule: "regex",
                    message: format!("`{field}` must match {summary} (regex `{pattern}`)"),
                    expected: json!({"pattern": pattern, "summary": summary}),
                    got: Some(json!(s)),
                }]),
                Err(_) => Ok(()), // bad pattern at code time — don't reject the user
            }
        }
        Rule::Length { field, min, max } => {
            let v = lookup(args, field);
            let Some(value) = v else { return Ok(()) };
            let n = if let Some(s) = value.as_str() {
                s.chars().count()
            } else if let Some(arr) = value.as_array() {
                arr.len()
            } else {
                return Err(vec![FieldViolation {
                    field: (*field).to_string(),
                    rule: "length",
                    message: format!("`{field}` must be a string or array"),
                    expected: json!({"min": min, "max": max}),
                    got: Some(value.clone()),
                }]);
            };
            let lo = min.unwrap_or(0);
            let hi = max.unwrap_or(usize::MAX);
            if n < lo || n > hi {
                return Err(vec![FieldViolation {
                    field: (*field).to_string(),
                    rule: "length",
                    message: format!(
                        "`{field}` length must be in [{}..{}], got {n}",
                        min.map(|x| x.to_string()).unwrap_or_else(|| "0".into()),
                        max.map(|x| x.to_string()).unwrap_or_else(|| "∞".into()),
                    ),
                    expected: json!({"min": min, "max": max}),
                    got: Some(json!(n)),
                }]);
            }
            Ok(())
        }
        Rule::ExactlyOne { fields } => {
            let present: Vec<&&str> = fields
                .iter()
                .filter(|f| lookup(args, f).is_some_and(|v| !v.is_null()))
                .collect();
            if present.len() == 1 {
                Ok(())
            } else {
                Err(vec![FieldViolation {
                    field: fields.join(", "),
                    rule: "exactly_one",
                    message: format!(
                        "exactly one of {fields:?} must be supplied; got {present:?}"
                    ),
                    expected: json!({"exactly_one": fields}),
                    got: Some(json!(present)),
                }])
            }
        }
        Rule::AtLeastOne { fields } => {
            let present: Vec<&&str> = fields
                .iter()
                .filter(|f| lookup(args, f).is_some_and(|v| !v.is_null()))
                .collect();
            if !present.is_empty() {
                Ok(())
            } else {
                Err(vec![FieldViolation {
                    field: fields.join(", "),
                    rule: "at_least_one",
                    message: format!("at least one of {fields:?} must be supplied"),
                    expected: json!({"at_least_one": fields}),
                    got: Some(json!([])),
                }])
            }
        }
        Rule::All(sub) => {
            // Every sub-rule must pass. Aggregate all failures so the LLM
            // sees the full list, not just the first.
            let mut out: Vec<FieldViolation> = Vec::new();
            for r in *sub {
                if let Err(mut v) = eval_one(r, args) {
                    out.append(&mut v);
                }
            }
            if out.is_empty() { Ok(()) } else { Err(out) }
        }
        Rule::Any(sub) => {
            // At least one branch must pass. Only surface failures when
            // every branch fails — and then surface ALL of them as a hint
            // about which paths were tried.
            let mut all_failures: Vec<FieldViolation> = Vec::new();
            for r in *sub {
                match eval_one(r, args) {
                    Ok(()) => return Ok(()),
                    Err(mut v) => all_failures.append(&mut v),
                }
            }
            Err(all_failures)
        }
        Rule::Not(inner) => {
            // The inner rule must NOT match. We swallow its failure list
            // and emit a single "not matched the negated rule" hint.
            match eval_one(inner, args) {
                Ok(()) => Err(vec![FieldViolation {
                    field: "<combinator>".into(),
                    rule: "not",
                    message: "negated rule unexpectedly matched".into(),
                    expected: json!({"not": format!("{inner:?}")}),
                    got: None,
                }]),
                Err(_) => Ok(()),
            }
        }
        Rule::Custom { eval, .. } => match eval(args) {
            Ok(()) => Ok(()),
            Err(v) => Err(vec![v]),
        },
    }
}

/// Dotted-path lookup. `"foo"` -> top-level; `"a.b"` -> nested; array
/// elements aren't supported by the path syntax yet (caller can write a
/// `Custom` rule for those rare cases).
fn lookup<'a>(args: &'a JsonObject, path: &str) -> Option<&'a Value> {
    let mut current: &Value = &Value::Object(args.clone().into_iter().collect());
    // The clone above is regrettable; the dispatcher hands us a JsonObject
    // (a rmcp alias) and we need a Value to descend through. The lookup
    // path is O(depth) which is tiny — depth 1 or 2 in practice.
    for seg in path.split('.') {
        current = current.get(seg)?;
    }
    // SAFETY-ish: we return a reference to the cloned object's child, but
    // Rust's borrow checker rejects that. So we return None — callers
    // should treat absence as "skip", and for primitives we re-read via
    // `args.get(...)`. Specialize for the common case:
    let _ = current;
    // Fast path: single-segment lookup against the original object.
    if !path.contains('.') {
        return args.get(path);
    }
    // Multi-segment: walk and clone once. Acceptable cost; deep paths are rare.
    let mut cur: Option<&Value> = args.get(path.split('.').next().unwrap_or(""));
    for seg in path.split('.').skip(1) {
        cur = cur.and_then(|v| v.get(seg));
    }
    cur
}

/// Render a rule tree as the JSON shape `describe_skill` returns.
pub fn rules_to_json(rules: &[Rule]) -> Value {
    Value::Array(rules.iter().map(rule_to_json).collect())
}

fn rule_to_json(r: &Rule) -> Value {
    match r {
        Rule::Range { field, min, max } => {
            json!({"rule": "range", "field": field, "min": min, "max": max})
        }
        Rule::OneOf { field, values } => {
            json!({"rule": "one_of", "field": field, "values": values})
        }
        Rule::Regex { field, pattern, summary } => {
            json!({"rule": "regex", "field": field, "pattern": pattern, "summary": summary})
        }
        Rule::Length { field, min, max } => {
            json!({"rule": "length", "field": field, "min": min, "max": max})
        }
        Rule::ExactlyOne { fields } => json!({"rule": "exactly_one", "fields": fields}),
        Rule::AtLeastOne { fields } => json!({"rule": "at_least_one", "fields": fields}),
        Rule::All(sub) => {
            json!({"rule": "all_of", "rules": sub.iter().map(rule_to_json).collect::<Vec<_>>()})
        }
        Rule::Any(sub) => {
            json!({"rule": "any_of", "rules": sub.iter().map(rule_to_json).collect::<Vec<_>>()})
        }
        Rule::Not(inner) => json!({"rule": "not", "inner": rule_to_json(inner)}),
        Rule::Custom { name, summary, .. } => {
            json!({"rule": "custom", "name": name, "summary": summary})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn args(json: Value) -> JsonObject {
        let Value::Object(m) = json else { panic!("not an object") };
        m.into_iter().collect::<Map<_, _>>()
    }

    #[test]
    fn range_in_bounds_passes() {
        let r = [Rule::Range { field: "code", min: Some(100.0), max: Some(599.0) }];
        let a = args(json!({"code": 200}));
        assert!(evaluate(&r, &a).is_pass());
    }

    #[test]
    fn range_out_of_bounds_fails() {
        let r = [Rule::Range { field: "code", min: Some(100.0), max: Some(599.0) }];
        let a = args(json!({"code": 700}));
        match evaluate(&r, &a) {
            ValidationResult::Fail(v) => {
                assert_eq!(v[0].rule, "range");
                assert_eq!(v[0].field, "code");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn one_of_passes() {
        let r = [Rule::OneOf { field: "kind", values: &["a", "b", "c"] }];
        let a = args(json!({"kind": "b"}));
        assert!(evaluate(&r, &a).is_pass());
    }

    #[test]
    fn one_of_rejects() {
        let r = [Rule::OneOf { field: "kind", values: &["a", "b"] }];
        let a = args(json!({"kind": "x"}));
        let res = evaluate(&r, &a);
        assert!(matches!(res, ValidationResult::Fail(_)));
    }

    #[test]
    fn exactly_one_enforced() {
        let r = [Rule::ExactlyOne { fields: &["a", "b"] }];
        // Both → fail.
        let two = args(json!({"a": 1, "b": 2}));
        assert!(matches!(evaluate(&r, &two), ValidationResult::Fail(_)));
        // None → fail.
        let zero = args(json!({}));
        assert!(matches!(evaluate(&r, &zero), ValidationResult::Fail(_)));
        // Exactly one → pass.
        let one = args(json!({"a": 1}));
        assert!(evaluate(&r, &one).is_pass());
    }

    #[test]
    fn any_or_passes_when_one_branch_does() {
        static SUB: &[Rule] = &[
            Rule::OneOf { field: "kind", values: &["x"] },
            Rule::Range { field: "code", min: Some(0.0), max: Some(10.0) },
        ];
        let r = [Rule::Any(SUB)];
        let a = args(json!({"kind": "wrong", "code": 5}));
        assert!(evaluate(&r, &a).is_pass());
    }

    #[test]
    fn any_or_fails_when_all_branches_do() {
        static SUB: &[Rule] = &[
            Rule::OneOf { field: "kind", values: &["x"] },
            Rule::Range { field: "code", min: Some(0.0), max: Some(10.0) },
        ];
        let r = [Rule::Any(SUB)];
        let a = args(json!({"kind": "wrong", "code": 100}));
        match evaluate(&r, &a) {
            ValidationResult::Fail(v) => assert_eq!(v.len(), 2),
            _ => panic!(),
        }
    }

    #[test]
    fn payload_shape() {
        let r = [Rule::Range { field: "p", min: Some(0.0), max: Some(100.0) }];
        let a = args(json!({"p": 150}));
        let p = evaluate(&r, &a).to_payload();
        assert!(p["validation_failed"].is_array());
        assert_eq!(p["validation_failed"][0]["field"], "p");
        assert_eq!(p["validation_failed"][0]["rule"], "range");
    }
}
