//! Trigonometry skill (local, no network): `trig_formula` / `trig_formula_list` for
//! named trig functions, inverses, angle conversions, and triangle relations. Angles
//! are in **degrees**. (`arithmetic_eval` also evaluates sin/cos/tan in radians.)

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::formula::{self, v, Args, Formula};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[rustfmt::skip]
static FORMULAS: LazyLock<Vec<Formula>> = LazyLock::new(|| {
    vec![
        Formula { id: "deg_to_rad", category: "trigonometry", summary: "Degrees → radians: rad = deg·π/180", inputs: vec![v("deg","deg")], out: v("rad","rad"), eval: |a| a["deg"].to_radians() },
        Formula { id: "rad_to_deg", category: "trigonometry", summary: "Radians → degrees: deg = rad·180/π", inputs: vec![v("rad","rad")], out: v("deg","deg"), eval: |a| a["rad"].to_degrees() },
        Formula { id: "sine", category: "trigonometry", summary: "Sine of an angle: sin(theta°)", inputs: vec![v("theta","deg")], out: v("value",""), eval: |a| a["theta"].to_radians().sin() },
        Formula { id: "cosine", category: "trigonometry", summary: "Cosine of an angle: cos(theta°)", inputs: vec![v("theta","deg")], out: v("value",""), eval: |a| a["theta"].to_radians().cos() },
        Formula { id: "tangent", category: "trigonometry", summary: "Tangent of an angle: tan(theta°)", inputs: vec![v("theta","deg")], out: v("value",""), eval: |a| a["theta"].to_radians().tan() },
        Formula { id: "arcsine", category: "trigonometry", summary: "Inverse sine: asin(x) in degrees (|x|≤1)", inputs: vec![v("x","")], out: v("theta","deg"), eval: |a| a["x"].asin().to_degrees() },
        Formula { id: "arccosine", category: "trigonometry", summary: "Inverse cosine: acos(x) in degrees (|x|≤1)", inputs: vec![v("x","")], out: v("theta","deg"), eval: |a| a["x"].acos().to_degrees() },
        Formula { id: "arctangent", category: "trigonometry", summary: "Inverse tangent: atan(x) in degrees", inputs: vec![v("x","")], out: v("theta","deg"), eval: |a| a["x"].atan().to_degrees() },
        Formula { id: "arctangent2", category: "trigonometry", summary: "Two-argument arctangent: atan2(y, x) in degrees", inputs: vec![v("y",""), v("x","")], out: v("theta","deg"), eval: |a| a["y"].atan2(a["x"]).to_degrees() },
        Formula { id: "law_of_sines_side", category: "trigonometry", summary: "Law of sines: a = b·sin(angle_a°)/sin(angle_b°)", inputs: vec![v("b",""), v("angle_a","deg"), v("angle_b","deg")], out: v("a",""), eval: |a| a["b"]*a["angle_a"].to_radians().sin()/a["angle_b"].to_radians().sin() },
        Formula { id: "law_of_cosines_angle", category: "trigonometry", summary: "Law of cosines (angle): C = acos((a²+b²-c²)/(2ab)) in degrees", inputs: vec![v("a",""), v("b",""), v("c","")], out: v("angle_c","deg"), eval: |a| ((a["a"].powi(2)+a["b"].powi(2)-a["c"].powi(2))/(2.0*a["a"]*a["b"])).acos().to_degrees() },
        Formula { id: "right_triangle_leg", category: "trigonometry", summary: "Right-triangle leg: b = √(c²-a²) (c hypotenuse)", inputs: vec![v("c",""), v("a","")], out: v("b",""), eval: |a| (a["c"].powi(2)-a["a"].powi(2)).sqrt() },
        Formula { id: "hypotenuse_from_angle", category: "trigonometry", summary: "Hypotenuse from opposite side & angle: c = opposite/sin(theta°)", inputs: vec![v("opposite",""), v("theta","deg")], out: v("c",""), eval: |a| a["opposite"]/a["theta"].to_radians().sin() },
        Formula { id: "arc_length", category: "trigonometry", summary: "Arc length: s = r·theta·π/180 (theta input in degrees; internally converted to radians)", inputs: vec![v("r",""), v("theta","deg")], out: v("s",""), eval: |a| a["r"]*a["theta"].to_radians() },
        Formula { id: "sector_area", category: "trigonometry", summary: "Circular sector area: A = ½·r²·theta·π/180 (theta input in degrees; internally converted to radians)", inputs: vec![v("r",""), v("theta","deg")], out: v("A",""), eval: |a| 0.5*a["r"].powi(2)*a["theta"].to_radians() },
    ]
});

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormulaArgs {
    /// Formula id (see `trig_formula_list`), e.g. `sine`, `law_of_cosines_angle`.
    name: String,
    /// Variable values, e.g. `{"theta": 30}`. Angles are in degrees.
    #[serde(default)]
    args: Args,
}

pub struct TrigFormula;
impl Skill for TrigFormula {
    fn name(&self) -> &'static str {
        "trig_formula"
    }
    fn description(&self) -> &'static str {
        "Compute a named trigonometry formula: sin/cos/tan and inverses (degrees), degree↔radian \
        conversion, law of sines/cosines, right-triangle relations, arc length, sector area. Pass \
        `name` (see trig_formula_list) and `args` as a {var: value} map; angles in degrees."
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Sine of 30°",
                args: r#"{"name": "sine", "args": {"theta": 30}}"#,
                note: Some("Returns 0.5. Angles are degrees, not radians."),
            },
            SkillExample {
                title: "Law of cosines (find angle)",
                args: r#"{"name": "law_of_cosines_angle", "args": {"a": 3, "b": 4, "c": 5}}"#,
                note: Some("Returns 90° (the right angle in a 3-4-5 triangle)."),
            },
            SkillExample {
                title: "Degrees to radians",
                args: r#"{"name": "deg_to_rad", "args": {"deg": 180}}"#,
                note: Some("Returns π."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute trig values in degrees without remembering the radian conversion.",
            "Apply law of sines / cosines or right-triangle relations to specific values.",
            "Convert between degrees and radians as a named operation.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListArgs {
    /// Optional id/equation substring filter.
    #[serde(default)]
    filter: Option<String>,
}

pub struct TrigFormulaList;
impl Skill for TrigFormulaList {
    fn name(&self) -> &'static str {
        "trig_formula_list"
    }
    fn description(&self) -> &'static str {
        "List the named trigonometry formulas (id, equation, signature). Feed an id to trig_formula."
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "List everything",
                args: r#"{}"#,
                note: Some("Returns id, equation, and signature for every trig formula."),
            },
            SkillExample {
                title: "Filter for inverses",
                args: r#"{"filter": "arc"}"#,
                note: Some("Substring match against id and summary."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Discover which formula id `trig_formula` accepts.",
            "Browse the available trig identities and triangle relations.",
        ]
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(TrigFormula), Box::new(TrigFormulaList)]
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
    fn trig_values() {
        assert!((run("sine", &[("theta", 30.0)]) - 0.5).abs() < 1e-9);
        assert!((run("cosine", &[("theta", 60.0)]) - 0.5).abs() < 1e-9);
        assert!((run("arcsine", &[("x", 0.5)]) - 30.0).abs() < 1e-9);
        assert!(
            (run(
                "law_of_cosines_angle",
                &[("a", 3.0), ("b", 4.0), ("c", 5.0)]
            ) - 90.0)
                .abs()
                < 1e-9
        );
        assert!((run("deg_to_rad", &[("deg", 180.0)]) - std::f64::consts::PI).abs() < 1e-9);
    }
}
