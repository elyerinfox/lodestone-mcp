//! `convert_units` skill (local, no network): convert a value between units of the
//! same kind — length, mass, volume, area, speed, time, data, and temperature.
//! Non-temperature units convert via a factor to a base unit; temperature is a
//! special affine case.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConvertUnitsArgs {
    /// The numeric value to convert.
    value: f64,
    /// Source unit, e.g. "km", "lb", "celsius", "GiB", "mph".
    from: String,
    /// Target unit (must be the same kind), e.g. "mi", "kg", "fahrenheit".
    to: String,
}

/// `(kind, factor-to-base)` for a non-temperature unit alias (lowercased).
/// Bases: length=m, mass=g, volume=l, area=m², speed=m/s, time=s, data=byte.
fn unit(alias: &str) -> Option<(&'static str, f64)> {
    Some(match alias {
        // length (base: metre)
        "mm" => ("length", 0.001),
        "cm" => ("length", 0.01),
        "m" | "meter" | "metre" => ("length", 1.0),
        "km" => ("length", 1000.0),
        "in" | "inch" => ("length", 0.0254),
        "ft" | "foot" | "feet" => ("length", 0.3048),
        "yd" | "yard" => ("length", 0.9144),
        "mi" | "mile" => ("length", 1609.344),
        "nmi" => ("length", 1852.0),
        // mass (base: gram)
        "mg" => ("mass", 0.001),
        "g" | "gram" => ("mass", 1.0),
        "kg" => ("mass", 1000.0),
        "t" | "tonne" => ("mass", 1_000_000.0),
        "oz" | "ounce" => ("mass", 28.349523125),
        "lb" | "lbs" | "pound" => ("mass", 453.59237),
        "st" | "stone" => ("mass", 6350.29318),
        // volume (base: litre)
        "ml" => ("volume", 0.001),
        "l" | "liter" | "litre" => ("volume", 1.0),
        "m3" => ("volume", 1000.0),
        "tsp" => ("volume", 0.00492892159375),
        "tbsp" => ("volume", 0.01478676478125),
        "cup" => ("volume", 0.2365882365),
        "pt" | "pint" => ("volume", 0.473176473),
        "qt" | "quart" => ("volume", 0.946352946),
        "gal" | "gallon" => ("volume", 3.785411784),
        "floz" => ("volume", 0.0295735295625),
        // area (base: m²)
        "mm2" => ("area", 1e-6),
        "cm2" => ("area", 1e-4),
        "m2" => ("area", 1.0),
        "km2" => ("area", 1e6),
        "ha" | "hectare" => ("area", 1e4),
        "acre" => ("area", 4046.8564224),
        "sqft" => ("area", 0.09290304),
        "sqin" => ("area", 0.00064516),
        "sqmi" => ("area", 2_589_988.110336),
        // speed (base: m/s)
        "m/s" | "mps" => ("speed", 1.0),
        "km/h" | "kmh" | "kph" => ("speed", 0.2777777777777778),
        "mph" => ("speed", 0.44704),
        "knot" | "kn" | "kt" => ("speed", 0.5144444444444445),
        "ft/s" | "fps" => ("speed", 0.3048),
        // time (base: second)
        "ns" => ("time", 1e-9),
        "us" => ("time", 1e-6),
        "ms" => ("time", 0.001),
        "s" | "sec" | "second" => ("time", 1.0),
        "min" | "minute" => ("time", 60.0),
        "h" | "hr" | "hour" => ("time", 3600.0),
        "day" | "d" => ("time", 86400.0),
        "week" | "wk" => ("time", 604800.0),
        // data (base: byte; decimal KB=1000 B, binary KiB=1024 B; bit=1/8 B)
        "bit" => ("data", 0.125),
        "byte" | "b" => ("data", 1.0),
        "kb" => ("data", 1000.0),
        "mb" => ("data", 1e6),
        "gb" => ("data", 1e9),
        "tb" => ("data", 1e12),
        "kib" => ("data", 1024.0),
        "mib" => ("data", 1_048_576.0),
        "gib" => ("data", 1_073_741_824.0),
        "tib" => ("data", 1_099_511_627_776.0),
        _ => return None,
    })
}

/// Temperature unit code, or `None` if `alias` isn't a temperature unit.
fn temp_unit(alias: &str) -> Option<char> {
    match alias {
        "c" | "celsius" | "°c" => Some('c'),
        "f" | "fahrenheit" | "°f" => Some('f'),
        "k" | "kelvin" | "°k" => Some('k'),
        _ => None,
    }
}

fn to_celsius(v: f64, u: char) -> f64 {
    match u {
        'f' => (v - 32.0) * 5.0 / 9.0,
        'k' => v - 273.15,
        _ => v,
    }
}

fn from_celsius(c: f64, u: char) -> f64 {
    match u {
        'f' => c * 9.0 / 5.0 + 32.0,
        'k' => c + 273.15,
        _ => c,
    }
}

fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    let r = (x * 1e9).round() / 1e9;
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{r}")
}

fn convert(value: f64, from: &str, to: &str) -> Result<f64, String> {
    let f = from.trim().to_ascii_lowercase();
    let t = to.trim().to_ascii_lowercase();

    if let (Some(fu), Some(tu)) = (temp_unit(&f), temp_unit(&t)) {
        return Ok(from_celsius(to_celsius(value, fu), tu));
    }
    if temp_unit(&f).is_some() || temp_unit(&t).is_some() {
        return Err("cannot convert between temperature and non-temperature units".into());
    }

    let (fk, ff) = unit(&f).ok_or_else(|| format!("unknown unit '{from}'"))?;
    let (tk, tf) = unit(&t).ok_or_else(|| format!("unknown unit '{to}'"))?;
    if fk != tk {
        return Err(format!(
            "incompatible units: '{from}' is {fk}, '{to}' is {tk}"
        ));
    }
    Ok(value * ff / tf)
}

pub struct ConvertUnits;
impl Skill for ConvertUnits {
    fn name(&self) -> &'static str {
        "convert_units"
    }
    fn description(&self) -> &'static str {
        "Convert a value between units of the same kind (local, no network): length (mm/cm/m/km/in/\
        ft/yd/mi), mass (mg/g/kg/t/oz/lb/st), volume (ml/l/m3/tsp/tbsp/cup/pt/qt/gal — **US \
        customary**, not imperial: US gal = 3.785 L vs imperial gal = 4.546 L), area \
        (cm2/m2/km2/ha/acre/sqft), speed (m/s/km/h/mph/knot), time (ms/s/min/h/day/week), data \
        (bit/byte/kb/mb/gb/kib/mib/gib — kb is decimal 10³, kib is binary 2¹⁰ per IEC 80000-13), \
        and temperature (celsius/fahrenheit/kelvin)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConvertUnitsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ConvertUnitsArgs>()?;
            let result = convert(args.value, &args.from, &args.to).map_err(invalid)?;
            Ok(text_result(format!(
                "{} {} = {} {}",
                fmt_num(args.value),
                args.from.trim(),
                fmt_num(result),
                args.to.trim()
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Length",
                args: r#"{"value": 5, "from": "mi", "to": "km"}"#,
                note: None,
            },
            SkillExample {
                title: "Temperature",
                args: r#"{"value": 100, "from": "celsius", "to": "fahrenheit"}"#,
                note: Some("Celsius/Fahrenheit/Kelvin are affine, not factor-based."),
            },
            SkillExample {
                title: "Decimal vs binary data",
                args: r#"{"value": 1, "from": "gib", "to": "gb"}"#,
                note: Some("`gib` is 2^30 bytes; `gb` is 10^9 bytes (IEC 80000-13)."),
            },
            SkillExample {
                title: "Speed",
                args: r#"{"value": 60, "from": "mph", "to": "m/s"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert between units of the same kind without risking a math error.",
            "Translate imperial measurements to metric (or back).",
            "Disambiguate decimal (GB) vs binary (GiB) storage sizes.",
        ]
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(ConvertUnits)]
}

#[cfg(test)]
mod tests {
    use super::convert;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn length_and_mass() {
        assert!(approx(convert(1.0, "km", "m").unwrap(), 1000.0));
        assert!(approx(convert(1.0, "kg", "g").unwrap(), 1000.0));
        assert!(approx(convert(1.0, "mi", "km").unwrap(), 1.609344));
    }

    #[test]
    fn temperature() {
        assert!(approx(convert(100.0, "c", "f").unwrap(), 212.0));
        assert!(approx(convert(0.0, "c", "k").unwrap(), 273.15));
        assert!(approx(convert(32.0, "f", "c").unwrap(), 0.0));
    }

    #[test]
    fn data_and_errors() {
        assert!(approx(convert(1.0, "kib", "byte").unwrap(), 1024.0));
        assert!(convert(1.0, "kg", "m").is_err()); // incompatible
        assert!(convert(1.0, "c", "kg").is_err()); // temp vs not
        assert!(convert(1.0, "frob", "m").is_err()); // unknown
    }
}
