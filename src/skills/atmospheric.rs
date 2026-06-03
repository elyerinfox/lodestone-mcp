//! Atmospheric science — ISA / US-1976 standard atmosphere, density altitude,
//! dewpoint via Magnus, wet-bulb globe temperature, NOAA SWPC space-weather Kp.
//! Pure math + one keyless web fetch; on by default.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, text_result};

const G0: f64 = 9.806_65;
const R_AIR: f64 = 287.052_8;
const RHO_SL: f64 = 1.225;
const T_SL: f64 = 288.15;
#[allow(dead_code)]
const P_SL: f64 = 101_325.0;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AltArgs {
    /// Geopotential altitude in meters (≤ 86000).
    altitude_m: f64,
}

pub struct AtmIsa;
impl Skill for AtmIsa {
    fn name(&self) -> &'static str {
        "atm_isa"
    }
    fn description(&self) -> &'static str {
        "ISA / US Standard Atmosphere 1976 — temperature, pressure, density at \
        a geopotential altitude. Returns `temp_k`, `pressure_pa`, `density_kg_m3`. \
        Layered closed-form valid 0 ≤ h ≤ 86 km."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AltArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<AltArgs>()?;
            let (t, p, rho) = isa(a.altitude_m);
            Ok(text_result(
                json!({ "temp_k": t, "pressure_pa": p, "density_kg_m3": rho }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Sea-level conditions",
                args: r#"{"altitude_m": 0}"#,
                note: Some(
                    "Returns the canonical ISA SL values (`temp_k: 288.15`, `pressure_pa: 101325`, `density_kg_m3: 1.225`).",
                ),
            },
            SkillExample {
                title: "Cruise altitude (FL360 ≈ 10 973 m)",
                args: r#"{"altitude_m": 10973}"#,
                note: Some("Tropospheric layer; expect ~218 K and ~24 kPa."),
            },
            SkillExample {
                title: "Stratosphere (20 km)",
                args: r#"{"altitude_m": 20000}"#,
                note: Some("Crosses into the isothermal layer at 11 km then a rising-lapse layer."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Look up ISA temperature / pressure / density at a known altitude.",
            "Feed a density value into a propulsion / aerodynamic calculation.",
            "Sanity-check sensor readings against the standard atmosphere.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "altitude_m",
            min: Some(-1000.0),
            max: Some(86_000.0),
        }]
    }
}

/// US Standard Atmosphere 1976 layered model. h_base, T_base, L (lapse K/m), p_base.
fn isa(h: f64) -> (f64, f64, f64) {
    let layers: [(f64, f64, f64, f64); 7] = [
        (0.0, 288.15, -0.0065, 101_325.0),
        (11_000.0, 216.65, 0.0, 22_632.06),
        (20_000.0, 216.65, 0.001, 5_474.889),
        (32_000.0, 228.65, 0.0028, 868.0187),
        (47_000.0, 270.65, 0.0, 110.9063),
        (51_000.0, 270.65, -0.0028, 66.93887),
        (71_000.0, 214.65, -0.002, 3.95642),
    ];
    let mut idx = 0;
    for (i, layer) in layers.iter().enumerate() {
        if h >= layer.0 {
            idx = i;
        }
    }
    let (h0, t0, l, p0) = layers[idx];
    let t = t0 + l * (h - h0);
    let p = if l.abs() < 1e-12 {
        p0 * (-G0 * (h - h0) / (R_AIR * t0)).exp()
    } else {
        p0 * (t / t0).powf(-G0 / (l * R_AIR))
    };
    let rho = p / (R_AIR * t);
    (t, p, rho)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DensityAltArgs {
    /// Station pressure (Pa) — sea-level equivalent or local barometric.
    pressure_pa: f64,
    /// Air temperature (°C).
    temp_c: f64,
    /// Dewpoint (°C), optional. When supplied, humidity correction applies.
    #[serde(default)]
    dewpoint_c: Option<f64>,
}

pub struct AtmDensityAltitude;
impl Skill for AtmDensityAltitude {
    fn name(&self) -> &'static str {
        "atm_density_altitude"
    }
    fn description(&self) -> &'static str {
        "Density altitude (m) — the ISA altitude at which the current air \
        density would be standard. Drives aircraft / rocket performance. \
        Includes virtual-temperature humidity correction when `dewpoint_c` \
        is supplied."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DensityAltArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DensityAltArgs>()?;
            let t_k = a.temp_c + 273.15;
            let t_v = if let Some(td) = a.dewpoint_c {
                let e = saturation_vapor_pressure(td);
                // Virtual temperature with humidity.
                t_k / (1.0 - 0.378 * e / a.pressure_pa)
            } else {
                t_k
            };
            let rho = a.pressure_pa / (R_AIR * t_v);
            // Invert ISA troposphere lapse for density altitude.
            let l = -0.0065;
            let exponent = -G0 / (l * R_AIR) - 1.0;
            let ratio = rho / RHO_SL;
            let h = (T_SL / l) * (1.0 - ratio.powf(1.0 / exponent));
            Ok(text_result(
                json!({ "density_altitude_m": h, "density_kg_m3": rho }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Hot summer day, sea-level pressure",
                args: r#"{"pressure_pa": 101325, "temp_c": 35}"#,
                note: Some("Returns density altitude in meters — the equivalent ISA altitude."),
            },
            SkillExample {
                title: "Humid conditions (virtual-temp correction)",
                args: r#"{"pressure_pa": 100000, "temp_c": 30, "dewpoint_c": 25}"#,
                note: Some("Supplying `dewpoint_c` triggers the humidity correction."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute density altitude for aircraft / rocket takeoff performance.",
            "Convert local pressure / temperature into an equivalent ISA altitude.",
            "Apply humidity correction to a hot-and-humid airfield computation.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "pressure_pa",
            min: Some(0.0),
            max: None,
        }]
    }
}

fn saturation_vapor_pressure(temp_c: f64) -> f64 {
    // Magnus formula: e_s(T) in Pa.
    611.2 * (17.62 * temp_c / (temp_c + 243.12)).exp()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DewpointArgs {
    /// Air temperature (°C).
    temp_c: f64,
    /// Relative humidity (0..100 %).
    rh_pct: f64,
}

pub struct AtmDewpoint;
impl Skill for AtmDewpoint {
    fn name(&self) -> &'static str {
        "atm_dewpoint"
    }
    fn description(&self) -> &'static str {
        "Dewpoint (°C) from temperature and relative humidity using the Magnus \
        formula. Reverse formula for relative humidity from dewpoint isn't \
        included here — `atm_relhumidity` is the sibling tool you want for \
        that (not yet wired)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DewpointArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DewpointArgs>()?;
            let alpha = ((a.rh_pct / 100.0).ln()) + (17.62 * a.temp_c) / (243.12 + a.temp_c);
            let td = 243.12 * alpha / (17.62 - alpha);
            Ok(text_result(json!({ "dewpoint_c": td }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Room conditions",
                args: r#"{"temp_c": 20, "rh_pct": 50}"#,
                note: Some("Returns dewpoint around 9.3 °C."),
            },
            SkillExample {
                title: "Hot and humid",
                args: r#"{"temp_c": 35, "rh_pct": 80}"#,
                note: Some(
                    "Dewpoint near skin temperature flags dangerous heat-stress conditions.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute dewpoint from temperature and relative humidity for HVAC / meteorology.",
            "Estimate frost / condensation threshold for an outdoor surface.",
            "Pre-compute the dewpoint input that `atm_density_altitude` accepts.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "rh_pct",
            min: Some(0.0),
            max: Some(100.0),
        }]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WbgtArgs {
    /// Air temperature (°C).
    temp_c: f64,
    /// Relative humidity (0..100 %).
    rh_pct: f64,
}

pub struct AtmWbgt;
impl Skill for AtmWbgt {
    fn name(&self) -> &'static str {
        "atm_wbgt"
    }
    fn description(&self) -> &'static str {
        "Indoor / shaded wet-bulb globe temperature estimate via the \
        Stull / Australian Bureau approximations: 0.7 · T_wb + 0.3 · T_air. \
        Outdoor (sun-exposed) WBGT needs radiant temperature input — for \
        which you'd want a black-globe thermometer. Use for heat-stress \
        risk thresholds (ACGIH TLVs)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WbgtArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<WbgtArgs>()?;
            // Stull (2011) wet-bulb temperature from T and RH.
            let t = a.temp_c;
            let rh = a.rh_pct;
            let tw = t * (0.151_977 * (rh + 8.313_659).sqrt()).atan() + (t + rh).atan()
                - (rh - 1.676_331).atan()
                + 0.003_918_38 * rh.powf(1.5) * (0.023_101 * rh).atan()
                - 4.686_035;
            let wbgt = 0.7 * tw + 0.3 * t;
            Ok(text_result(
                json!({ "wbgt_c": wbgt, "wet_bulb_c": tw }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Warehouse heat-stress check",
                args: r#"{"temp_c": 32, "rh_pct": 65}"#,
                note: Some("Returns indoor WBGT and the underlying wet-bulb temperature."),
            },
            SkillExample {
                title: "Mild office conditions",
                args: r#"{"temp_c": 22, "rh_pct": 40}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate indoor / shaded WBGT for occupational heat-stress decisions.",
            "Screen ACGIH TLV thresholds before scheduling outdoor / hot work.",
            "Get the wet-bulb temperature alongside WBGT in one call.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "rh_pct",
            min: Some(0.0),
            max: Some(100.0),
        }]
    }
}

pub struct AtmSpaceWeatherKp;
impl Skill for AtmSpaceWeatherKp {
    fn name(&self) -> &'static str {
        "atm_space_weather_kp"
    }
    fn description(&self) -> &'static str {
        "Live planetary K-index (3-hour resolution, last 24h) from NOAA SWPC. \
        Higher Kp → stronger geomagnetic disturbance → degraded HF radio + \
        auroral activity. Returns the raw GFZ-Potsdam-style array; the last \
        row is the most recent."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            // Shared over the constellation by canonical key so peers can
            // serve the same 3-hour Kp window without each node re-fetching.
            let key = "swpc|planetary-k-index".to_string();
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let r = server
                .http
                .get("https://services.swpc.noaa.gov/products/noaa-planetary-k-index.json")
                .send()
                .await
                .map_err(|e| internal(anyhow::anyhow!(e)))?;
            if !r.status().is_success() {
                return Err(internal(anyhow::anyhow!(
                    "SWPC returned status {}",
                    r.status()
                )));
            }
            let body = r.text().await.map_err(|e| internal(anyhow::anyhow!(e)))?;
            let body = truncate_chars(&body, server.max_chars);
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Fetch current Kp",
            args: r#"{}"#,
            note: Some(
                "Returns the SWPC 3-hour planetary K-index array; the last row is the most recent.",
            ),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check current geomagnetic disturbance level before HF radio operations.",
            "Decide whether auroral activity is likely tonight at high latitudes.",
            "Pull the raw NOAA SWPC Kp window for downstream analysis.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(AtmIsa),
        Box::new(AtmDensityAltitude),
        Box::new(AtmDewpoint),
        Box::new(AtmWbgt),
        Box::new(AtmSpaceWeatherKp),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isa_sea_level() {
        let (t, p, rho) = isa(0.0);
        assert!((t - T_SL).abs() < 0.01);
        assert!((p - P_SL).abs() < 1.0);
        assert!((rho - RHO_SL).abs() < 1e-3);
    }

    #[test]
    fn isa_11km() {
        // Tropopause.
        let (t, p, _) = isa(11_000.0);
        assert!((t - 216.65).abs() < 0.1);
        assert!((p - 22_632.0).abs() < 50.0);
    }

    #[test]
    fn dewpoint_50pct() {
        // 20 °C / 50% RH → roughly 9.3 °C.
        let alpha = ((50.0_f64 / 100.0).ln()) + (17.62 * 20.0) / (243.12 + 20.0);
        let td = 243.12 * alpha / (17.62 - alpha);
        assert!((td - 9.3).abs() < 0.2);
    }
}
