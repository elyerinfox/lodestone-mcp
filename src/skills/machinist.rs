//! Machinist / mechanical-engineering tool family — cutting kinematics,
//! material removal rate, specific cutting energy (Kienzle), surface finish,
//! beam deflection, stress/strain, bolt torque, vendored UNC/UNF/metric
//! thread + tap-drill tables, material properties, and hardness conversions.
//! Pure-Rust, on by default. No network; tables are vendored, so no
//! constellation cache is needed here.
//!
//! ## Source citations
//!
//! - **Cutting kinematics**: *Machinery's Handbook* 31st ed. (Industrial
//!   Press, 2020), "Cutting Speeds and Feeds" section.
//! - **Specific cutting energy (Kienzle)**: Sandvik Coromant,
//!   *Specific Cutting Force k_c*
//!   (<https://www.sandvik.coromant.com/en-us/knowledge/materials/specific-cutting-force>).
//!   k_c = k_c1 · h_m^(−m_c).
//! - **Surface finish (theoretical Ra in turning)**: ISO 4287:1997,
//!   *Geometrical Product Specifications — Surface Texture*.
//! - **Beam deflection / cross-section inertias**: R. G. Budynas &
//!   J. K. Nisbett, *Shigley's Mechanical Engineering Design*, 11th ed.,
//!   McGraw-Hill 2020, Table A-9.
//! - **Bolt torque**: Shigley's, Eq. 8-27 (T = K·d·F_i); preload Eq. 8-31/32
//!   (F_i = 0.75·F_p reusable, 0.90·F_p permanent). Nut-factor K table:
//!   Shigley's Table 8-15 (dry as-received K = 0.30 — different from the
//!   often-quoted 0.20 lubricated default).
//! - **Threads and tap drills**: ASME B1.1 (UNC/UNF) and ISO 261 (metric).
//!   75 % engagement values, *Machinery's Handbook* 31st ed.
//! - **Material properties**: MatWeb data sheets and ASM Handbook, Vol 1-2,
//!   typical certified values for each named alloy.
//! - **Hardness conversion**: ASTM E140-12b Table 1 (non-austenitic
//!   steels). Polynomial fit valid for HRC 20-60.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// mach_cutting_speed: V → RPM (metric and imperial).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CutSpeedArgs {
    /// Cutting speed: m/min (when `units == "metric"`) or surface feet per
    /// minute when `units == "imperial"`.
    v: f64,
    /// Tool / workpiece diameter — mm (metric) or inches (imperial).
    diameter: f64,
    /// `metric` (default) or `imperial`.
    #[serde(default)]
    units: Option<String>,
}

pub struct MachCuttingSpeed;
impl Skill for MachCuttingSpeed {
    fn name(&self) -> &'static str {
        "mach_cutting_speed"
    }
    fn description(&self) -> &'static str {
        "Spindle RPM from cutting speed: N = 1000·V/(π·D) (metric, V in \
        m/min, D in mm) or N = 12·V/(π·D) (imperial, V in sfm, D in \
        inches). Citation: Machinery's Handbook 31 e., 'Cutting Speeds & \
        Feeds'."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CutSpeedArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CutSpeedArgs>()?;
            if a.diameter <= 0.0 {
                return Err(invalid("diameter must be > 0"));
            }
            let units = a.units.as_deref().unwrap_or("metric").to_ascii_lowercase();
            let rpm = match units.as_str() {
                "metric" => 1000.0 * a.v / (std::f64::consts::PI * a.diameter),
                "imperial" => 12.0 * a.v / (std::f64::consts::PI * a.diameter),
                other => return Err(invalid(format!("unknown units '{other}'"))),
            };
            Ok(text_result(json!({ "rpm": rpm }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Metric — 6061 face mill",
                args: r#"{"v": 300, "diameter": 50, "units": "metric"}"#,
                note: Some("V in m/min, D in mm; returns RPM."),
            },
            SkillExample {
                title: "Imperial — steel SFM",
                args: r#"{"v": 100, "diameter": 0.5, "units": "imperial"}"#,
                note: Some("V in sfm, D in inches."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert a recommended cutting speed (m/min or sfm) into spindle RPM.",
            "Set up speeds-and-feeds for a turning or milling job.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_feed_rate: F = f_z · z · N.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FeedArgs {
    /// Feed per tooth / chip load (mm/tooth or inch/tooth).
    feed_per_tooth: f64,
    /// Number of flutes / teeth.
    teeth: u32,
    /// Spindle speed (RPM).
    rpm: f64,
}

pub struct MachFeedRate;
impl Skill for MachFeedRate {
    fn name(&self) -> &'static str {
        "mach_feed_rate"
    }
    fn description(&self) -> &'static str {
        "Linear feed rate F = f_z · z · N. Units are whatever you supply for \
        chip load (mm/tooth → mm/min, inch/tooth → inch/min). Citation: \
        Machinery's Handbook 31 e."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FeedArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<FeedArgs>()?;
            let f = a.feed_per_tooth * a.teeth as f64 * a.rpm;
            Ok(text_result(json!({ "feed_per_min": f }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "4-flute end mill",
                args: r#"{"feed_per_tooth": 0.05, "teeth": 4, "rpm": 1800}"#,
                note: Some("Returns feed in mm/min (chip load was mm/tooth)."),
            },
            SkillExample {
                title: "Drill — 2 flutes",
                args: r#"{"feed_per_tooth": 0.1, "teeth": 2, "rpm": 1200}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute linear feed from chip load, flute count, and RPM.",
            "Verify the F value to enter into a CAM post or controller.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_mrr_milling: a_e · a_p · F.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MrrArgs {
    /// Radial width of cut, a_e (mm).
    a_e_mm: f64,
    /// Axial depth of cut, a_p (mm).
    a_p_mm: f64,
    /// Feed rate (mm/min).
    feed_mm_min: f64,
}

pub struct MachMrrMilling;
impl Skill for MachMrrMilling {
    fn name(&self) -> &'static str {
        "mach_mrr_milling"
    }
    fn description(&self) -> &'static str {
        "Material removal rate (milling): MRR = a_e · a_p · F. Returns MRR \
        in cm³/min (after unit reconciliation). Citation: Machinery's \
        Handbook 31 e."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MrrArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<MrrArgs>()?;
            // mm³/min ÷ 1000 → cm³/min.
            let mrr = a.a_e_mm * a.a_p_mm * a.feed_mm_min / 1000.0;
            Ok(text_result(json!({ "mrr_cm3_min": mrr }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Slotting cut",
                args: r#"{"a_e_mm": 10, "a_p_mm": 5, "feed_mm_min": 600}"#,
                note: Some("Returns MRR in cm³/min."),
            },
            SkillExample {
                title: "Light finish pass",
                args: r#"{"a_e_mm": 2, "a_p_mm": 0.5, "feed_mm_min": 1200}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute milling MRR for power / cycle-time estimates.",
            "Compare two strategies' chip-removal throughput.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_cutting_power: Kienzle, k_c = k_c1 · h_m^(−m_c).
// ---------------------------------------------------------------------------

struct CuttingMaterial {
    key: &'static str,
    kc1: f64,
    mc: f64,
    name: &'static str,
}

const CUTTING_MATERIALS: &[CuttingMaterial] = &[
    CuttingMaterial {
        key: "al-6061",
        kc1: 500.0,
        mc: 0.25,
        name: "Aluminum 6061 (Sandvik N group)",
    },
    CuttingMaterial {
        key: "steel-1020",
        kc1: 1900.0,
        mc: 0.25,
        name: "Carbon steel 1020 / C45 (Sandvik P)",
    },
    CuttingMaterial {
        key: "ss-304",
        kc1: 1900.0,
        mc: 0.21,
        name: "Stainless 304 (Sandvik M)",
    },
    CuttingMaterial {
        key: "cast-iron",
        kc1: 950.0,
        mc: 0.28,
        name: "Gray cast iron (Sandvik K)",
    },
    CuttingMaterial {
        key: "ti-6al4v",
        kc1: 1400.0,
        mc: 0.23,
        name: "Ti-6Al-4V (Sandvik S)",
    },
];

fn find_cutting_material(key: &str) -> Option<&'static CuttingMaterial> {
    let k = key.trim().to_ascii_lowercase();
    CUTTING_MATERIALS.iter().find(|m| m.key == k)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PowerArgs {
    /// Material removal rate, cm³/min.
    mrr_cm3_min: f64,
    /// Mean chip thickness (mm). For Kienzle k_c(h_m).
    h_m_mm: f64,
    /// One of `al-6061`, `steel-1020`, `ss-304`, `cast-iron`, `ti-6al4v`.
    material: String,
}

pub struct MachCuttingPower;
impl Skill for MachCuttingPower {
    fn name(&self) -> &'static str {
        "mach_cutting_power"
    }
    fn description(&self) -> &'static str {
        "Cutting power via the Kienzle model: P (kW) = MRR(cm³/min) · k_c \
        (N/mm²) / 60 000, where k_c = k_c1 · h_m^(−m_c). Vendored k_c1 / m_c \
        for Aluminum 6061, carbon steel 1020/C45, stainless 304, gray cast \
        iron, and Ti-6Al-4V (Sandvik Coromant nominal mid-range)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PowerArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PowerArgs>()?;
            if a.h_m_mm <= 0.0 {
                return Err(invalid("h_m_mm must be > 0"));
            }
            let m = find_cutting_material(&a.material)
                .ok_or_else(|| invalid(format!("unknown material '{}'", a.material)))?;
            let k_c = m.kc1 * a.h_m_mm.powf(-m.mc);
            // P [W] = MRR [mm³/min] · k_c [N/mm²] / 60 → P [kW] = above / 1000.
            let p_kw = a.mrr_cm3_min * 1000.0 * k_c / 60.0 / 1000.0;
            Ok(text_result(
                json!({
                    "power_kw": p_kw,
                    "k_c_n_per_mm2": k_c,
                    "material": m.name,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Aluminum at MRR 30 cm³/min",
                args: r#"{"mrr_cm3_min": 30, "h_m_mm": 0.1, "material": "al-6061"}"#,
                note: Some("Returns spindle power in kW and computed k_c."),
            },
            SkillExample {
                title: "Carbon steel finish",
                args: r#"{"mrr_cm3_min": 8, "h_m_mm": 0.08, "material": "steel-1020"}"#,
                note: None,
            },
            SkillExample {
                title: "Titanium roughing",
                args: r#"{"mrr_cm3_min": 5, "h_m_mm": 0.15, "material": "ti-6al4v"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate required spindle power for a cutting recipe.",
            "Check whether a job fits a machine's power envelope.",
            "Compare cutting energy across materials at the same MRR.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_surface_finish_turning — theoretical Ra = f² / (32·r_n).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SurfArgs {
    /// Feed per revolution (mm/rev).
    feed_mm_rev: f64,
    /// Insert nose radius (mm).
    nose_radius_mm: f64,
}

pub struct MachSurfaceFinishTurning;
impl Skill for MachSurfaceFinishTurning {
    fn name(&self) -> &'static str {
        "mach_surface_finish_turning"
    }
    fn description(&self) -> &'static str {
        "Theoretical surface roughness in turning (ISO 4287): Ra ≈ f²/(32·r_n), \
        Rt ≈ f²/(8·r_n). Real Ra is typically 20-50 % higher due to vibration \
        and built-up edge — treat as a lower bound."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SurfArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SurfArgs>()?;
            if a.nose_radius_mm <= 0.0 {
                return Err(invalid("nose_radius_mm must be > 0"));
            }
            let ra_mm = a.feed_mm_rev.powi(2) / (32.0 * a.nose_radius_mm);
            let rt_mm = a.feed_mm_rev.powi(2) / (8.0 * a.nose_radius_mm);
            Ok(text_result(
                json!({
                    "ra_um": ra_mm * 1000.0,
                    "rt_um": rt_mm * 1000.0,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Standard insert finish",
                args: r#"{"feed_mm_rev": 0.1, "nose_radius_mm": 0.4}"#,
                note: Some("Returns Ra and Rt in micrometres."),
            },
            SkillExample {
                title: "Fine finish, larger nose",
                args: r#"{"feed_mm_rev": 0.05, "nose_radius_mm": 0.8}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate the lower-bound surface roughness for a turning operation.",
            "Pick a feed/nose-radius pair to hit a target Ra.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_beam_deflection — Shigley table A-9.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BeamArgs {
    /// One of `cantilever_end_load`, `cantilever_udl`, `simple_center_load`,
    /// `simple_udl`.
    case: String,
    /// Load — Point load P (N) or distributed load w (N/m), depending on case.
    load: f64,
    /// Length L (m).
    length_m: f64,
    /// Young's modulus E (Pa).
    e_pa: f64,
    /// Area moment of inertia I (m⁴).
    i_m4: f64,
}

pub struct MachBeamDeflection;
impl Skill for MachBeamDeflection {
    fn name(&self) -> &'static str {
        "mach_beam_deflection"
    }
    fn description(&self) -> &'static str {
        "Maximum deflection of a uniform Euler-Bernoulli beam (small-\
        deflection theory, Shigley's table A-9). Cases: cantilever with \
        end-point load (δ = PL³/3EI), cantilever with uniform load (wL⁴/8EI), \
        simply-supported with center point load (PL³/48EI), simply-supported \
        with uniform load (5wL⁴/384EI). Use `mach_section_inertia` to compute \
        I for rectangular or round cross-sections."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BeamArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BeamArgs>()?;
            if a.e_pa <= 0.0 || a.i_m4 <= 0.0 || a.length_m <= 0.0 {
                return Err(invalid("E, I, L must be > 0"));
            }
            let l3 = a.length_m.powi(3);
            let l4 = a.length_m.powi(4);
            let ei = a.e_pa * a.i_m4;
            let delta = match a.case.to_ascii_lowercase().as_str() {
                "cantilever_end_load" => a.load * l3 / (3.0 * ei),
                "cantilever_udl" => a.load * l4 / (8.0 * ei),
                "simple_center_load" => a.load * l3 / (48.0 * ei),
                "simple_udl" => 5.0 * a.load * l4 / (384.0 * ei),
                other => return Err(invalid(format!("unknown case '{other}'"))),
            };
            Ok(text_result(json!({ "delta_m": delta }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Cantilever with end load",
                args: r#"{"case": "cantilever_end_load", "load": 100, "length_m": 0.5, "e_pa": 2.0e11, "i_m4": 1.0e-9}"#,
                note: Some("Steel, 500 mm, 100 N tip load."),
            },
            SkillExample {
                title: "Simply-supported with center load",
                args: r#"{"case": "simple_center_load", "load": 500, "length_m": 1.0, "e_pa": 6.89e10, "i_m4": 5.0e-9}"#,
                note: None,
            },
            SkillExample {
                title: "Cantilever with UDL",
                args: r#"{"case": "cantilever_udl", "load": 200, "length_m": 0.8, "e_pa": 2.0e11, "i_m4": 4.9e-10}"#,
                note: Some("`load` is distributed load w (N/m) for UDL cases."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute max deflection for a uniform beam under a standard load case.",
            "Size a beam cross-section against a deflection budget.",
            "Cross-check a hand calc against Shigley table A-9.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_section_inertia — rectangle and round.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InertiaArgs {
    /// `rect` (b, h) or `round` (d).
    shape: String,
    /// Rectangle width b (m).
    #[serde(default)]
    b_m: Option<f64>,
    /// Rectangle height h (m).
    #[serde(default)]
    h_m: Option<f64>,
    /// Round diameter d (m).
    #[serde(default)]
    d_m: Option<f64>,
}

pub struct MachSectionInertia;
impl Skill for MachSectionInertia {
    fn name(&self) -> &'static str {
        "mach_section_inertia"
    }
    fn description(&self) -> &'static str {
        "Area moment of inertia I for two common cross sections (about the \
        neutral axis through the centroid): rectangle I = b·h³/12; solid \
        round I = π·d⁴/64. Citation: Shigley's, Table A-18."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InertiaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<InertiaArgs>()?;
            let i = match a.shape.to_ascii_lowercase().as_str() {
                "rect" => {
                    let b = a.b_m.ok_or_else(|| invalid("rect needs b_m"))?;
                    let h = a.h_m.ok_or_else(|| invalid("rect needs h_m"))?;
                    b * h.powi(3) / 12.0
                }
                "round" => {
                    let d = a.d_m.ok_or_else(|| invalid("round needs d_m"))?;
                    std::f64::consts::PI * d.powi(4) / 64.0
                }
                other => return Err(invalid(format!("unknown shape '{other}'"))),
            };
            Ok(text_result(json!({ "i_m4": i }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Rectangular bar",
                args: r#"{"shape": "rect", "b_m": 0.05, "h_m": 0.025}"#,
                note: Some("50×25 mm; I about the strong axis (h cubed)."),
            },
            SkillExample {
                title: "Round shaft",
                args: r#"{"shape": "round", "d_m": 0.020}"#,
                note: Some("20 mm diameter solid round."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Get area moment of inertia I for a rect or round cross-section.",
            "Feed I into `mach_beam_deflection` for a stiffness calc.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_stress_strain.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StressArgs {
    /// Force (N).
    force_n: f64,
    /// Cross-sectional area (m²).
    area_m2: f64,
    /// Young's modulus (Pa). When provided the tool also returns strain.
    #[serde(default)]
    e_pa: Option<f64>,
}

pub struct MachStressStrain;
impl Skill for MachStressStrain {
    fn name(&self) -> &'static str {
        "mach_stress_strain"
    }
    fn description(&self) -> &'static str {
        "Axial engineering stress σ = F/A. With Young's modulus E, the \
        elastic strain ε = σ/E is also returned."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StressArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<StressArgs>()?;
            if a.area_m2 <= 0.0 {
                return Err(invalid("area_m2 must be > 0"));
            }
            let sigma = a.force_n / a.area_m2;
            let mut out = json!({ "stress_pa": sigma });
            if let Some(e) = a.e_pa {
                if e > 0.0 {
                    let eps = sigma / e;
                    out["strain"] = json!(eps);
                }
            }
            Ok(text_result(out.to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Stress only",
                args: r#"{"force_n": 10000, "area_m2": 0.0001}"#,
                note: Some("Returns stress in Pa."),
            },
            SkillExample {
                title: "Stress + strain (with E)",
                args: r#"{"force_n": 5000, "area_m2": 0.00005, "e_pa": 2.0e11}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute axial engineering stress σ = F/A.",
            "Get elastic strain ε when Young's modulus is known.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_bolt_torque — T = K·d·F, Shigley.
// ---------------------------------------------------------------------------

struct NutFactor {
    key: &'static str,
    k: f64,
    note: &'static str,
}

const NUT_FACTORS: &[NutFactor] = &[
    NutFactor {
        key: "dry",
        k: 0.30,
        note: "as-received, Shigley Table 8-15",
    },
    NutFactor {
        key: "zinc",
        k: 0.20,
        note: "zinc plated",
    },
    NutFactor {
        key: "lubricated",
        k: 0.18,
        note: "general lubrication",
    },
    NutFactor {
        key: "cadmium",
        k: 0.16,
        note: "cadmium plated",
    },
    NutFactor {
        key: "anti-seize",
        k: 0.12,
        note: "copper/nickel anti-seize paste",
    },
    NutFactor {
        key: "ptfe",
        k: 0.10,
        note: "PTFE / moly disulfide",
    },
    NutFactor {
        key: "default",
        k: 0.20,
        note: "safe default in absence of data",
    },
];

fn find_nut_factor(key: &str) -> Option<&'static NutFactor> {
    let k = key.trim().to_ascii_lowercase();
    NUT_FACTORS.iter().find(|n| n.key == k)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BoltArgs {
    /// Bolt nominal diameter (m).
    diameter_m: f64,
    /// Desired preload (N).
    preload_n: f64,
    /// One of `dry`, `zinc`, `lubricated`, `cadmium`, `anti-seize`, `ptfe`,
    /// `default`.
    condition: String,
}

pub struct MachBoltTorque;
impl Skill for MachBoltTorque {
    fn name(&self) -> &'static str {
        "mach_bolt_torque"
    }
    fn description(&self) -> &'static str {
        "Bolt installation torque T = K·d·F (Shigley Eq. 8-27) for a given \
        preload F and nut-factor condition. Vendored K from Shigley Table \
        8-15: dry 0.30, zinc 0.20, lubricated 0.18, cadmium 0.16, anti-seize \
        0.12, PTFE 0.10. (Popular shorthand of 0.20 for dry differs from \
        Shigley — surfaced explicitly to avoid surprise.) Recommended \
        preload for reusable connections: F = 0.75·F_proof (Eq. 8-31)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BoltArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BoltArgs>()?;
            if a.diameter_m <= 0.0 || a.preload_n <= 0.0 {
                return Err(invalid("diameter and preload must be > 0"));
            }
            let nf = find_nut_factor(&a.condition)
                .ok_or_else(|| invalid(format!("unknown condition '{}'", a.condition)))?;
            let torque_nm = nf.k * a.diameter_m * a.preload_n;
            Ok(text_result(
                json!({
                    "torque_nm": torque_nm,
                    "k": nf.k,
                    "condition": nf.note,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "M10 dry, 25 kN preload",
                args: r#"{"diameter_m": 0.010, "preload_n": 25000, "condition": "dry"}"#,
                note: Some("Returns installation torque in N·m."),
            },
            SkillExample {
                title: "Lubricated 1/4-20",
                args: r#"{"diameter_m": 0.00635, "preload_n": 5000, "condition": "lubricated"}"#,
                note: None,
            },
            SkillExample {
                title: "Anti-seize on stainless",
                args: r#"{"diameter_m": 0.012, "preload_n": 30000, "condition": "anti-seize"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute target torque for a bolted joint at a given preload.",
            "Compare lubrication conditions' effect on installation torque.",
            "Sanity-check a torque spec against Shigley's nut-factor table.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_thread_specs and mach_tap_drill — vendored ASME B1.1 / ISO 261 tables.
// ---------------------------------------------------------------------------

struct ThreadSpec {
    name: &'static str,
    standard: &'static str,
    pitch_mm: f64,
    major_mm: f64,
    minor_mm: f64,
    tap_drill_mm: f64,
    tap_drill_name: &'static str,
}

const THREADS: &[ThreadSpec] = &[
    // UNC — ASME B1.1, 75 % engagement (Machinery's Handbook 31 e.).
    ThreadSpec {
        name: "1/4-20",
        standard: "UNC",
        pitch_mm: 25.4 / 20.0,
        major_mm: 6.350,
        minor_mm: 4.976,
        tap_drill_mm: 5.105,
        tap_drill_name: "#7 (0.2010 in)",
    },
    ThreadSpec {
        name: "5/16-18",
        standard: "UNC",
        pitch_mm: 25.4 / 18.0,
        major_mm: 7.938,
        minor_mm: 6.411,
        tap_drill_mm: 6.528,
        tap_drill_name: "F (0.2570 in)",
    },
    ThreadSpec {
        name: "3/8-16",
        standard: "UNC",
        pitch_mm: 25.4 / 16.0,
        major_mm: 9.525,
        minor_mm: 7.805,
        tap_drill_mm: 7.938,
        tap_drill_name: "5/16 (0.3125 in)",
    },
    ThreadSpec {
        name: "1/2-13",
        standard: "UNC",
        pitch_mm: 25.4 / 13.0,
        major_mm: 12.700,
        minor_mm: 10.584,
        tap_drill_mm: 10.716,
        tap_drill_name: "27/64 (0.4219 in)",
    },
    // ISO metric coarse — ISO 261; tap drill ≈ D − P.
    ThreadSpec {
        name: "M3",
        standard: "ISO_M_coarse",
        pitch_mm: 0.5,
        major_mm: 3.000,
        minor_mm: 2.459,
        tap_drill_mm: 2.50,
        tap_drill_name: "2.50 mm",
    },
    ThreadSpec {
        name: "M4",
        standard: "ISO_M_coarse",
        pitch_mm: 0.7,
        major_mm: 4.000,
        minor_mm: 3.242,
        tap_drill_mm: 3.30,
        tap_drill_name: "3.30 mm",
    },
    ThreadSpec {
        name: "M5",
        standard: "ISO_M_coarse",
        pitch_mm: 0.8,
        major_mm: 5.000,
        minor_mm: 4.134,
        tap_drill_mm: 4.20,
        tap_drill_name: "4.20 mm",
    },
    ThreadSpec {
        name: "M6",
        standard: "ISO_M_coarse",
        pitch_mm: 1.0,
        major_mm: 6.000,
        minor_mm: 4.917,
        tap_drill_mm: 5.00,
        tap_drill_name: "5.00 mm",
    },
    ThreadSpec {
        name: "M8",
        standard: "ISO_M_coarse",
        pitch_mm: 1.25,
        major_mm: 8.000,
        minor_mm: 6.647,
        tap_drill_mm: 6.80,
        tap_drill_name: "6.80 mm",
    },
    ThreadSpec {
        name: "M10",
        standard: "ISO_M_coarse",
        pitch_mm: 1.5,
        major_mm: 10.000,
        minor_mm: 8.376,
        tap_drill_mm: 8.50,
        tap_drill_name: "8.50 mm",
    },
    ThreadSpec {
        name: "M12",
        standard: "ISO_M_coarse",
        pitch_mm: 1.75,
        major_mm: 12.000,
        minor_mm: 10.106,
        tap_drill_mm: 10.20,
        tap_drill_name: "10.20 mm",
    },
];

fn find_thread(name: &str) -> Option<&'static ThreadSpec> {
    let n = name.trim();
    THREADS.iter().find(|t| t.name.eq_ignore_ascii_case(n))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ThreadArgs {
    /// Thread designation. Examples: `1/4-20`, `M6`, `M10`.
    thread: String,
}

pub struct MachThreadSpec;
impl Skill for MachThreadSpec {
    fn name(&self) -> &'static str {
        "mach_thread_spec"
    }
    fn description(&self) -> &'static str {
        "Look up a UNC (ASME B1.1) or ISO metric coarse (ISO 261) thread by \
        designation. Returns standard, pitch (mm), major + minor diameter \
        (mm), and the matching 75 %-engagement tap-drill diameter and named \
        size. Tables vendored from Machinery's Handbook 31 e."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ThreadArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ThreadArgs>()?;
            let t = find_thread(&a.thread)
                .ok_or_else(|| invalid(format!("unknown thread '{}'", a.thread)))?;
            Ok(text_result(
                json!({
                    "thread": t.name,
                    "standard": t.standard,
                    "pitch_mm": t.pitch_mm,
                    "major_diameter_mm": t.major_mm,
                    "minor_diameter_mm": t.minor_mm,
                    "tap_drill_mm": t.tap_drill_mm,
                    "tap_drill_name": t.tap_drill_name,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "UNC inch",
                args: r#"{"thread": "1/4-20"}"#,
                note: Some("Returns pitch, major/minor diameters, and tap-drill size."),
            },
            SkillExample {
                title: "ISO metric coarse",
                args: r#"{"thread": "M6"}"#,
                note: None,
            },
            SkillExample {
                title: "Larger metric",
                args: r#"{"thread": "M12"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Look up tap-drill size for a UNC or ISO metric thread.",
            "Get pitch and minor diameter for a thread engagement calc.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_material — typical mechanical properties.
// ---------------------------------------------------------------------------

struct Material {
    key: &'static str,
    name: &'static str,
    yield_mpa: f64,
    ultimate_mpa: f64,
    density_g_cm3: f64,
    e_gpa: f64,
    nu: f64,
}

const MATERIALS: &[Material] = &[
    Material {
        key: "1018-cd",
        name: "AISI 1018 cold-drawn steel",
        yield_mpa: 370.0,
        ultimate_mpa: 440.0,
        density_g_cm3: 7.87,
        e_gpa: 205.0,
        nu: 0.29,
    },
    Material {
        key: "4140-qt",
        name: "AISI 4140 Q&T (425 °C)",
        yield_mpa: 655.0,
        ultimate_mpa: 1020.0,
        density_g_cm3: 7.85,
        e_gpa: 205.0,
        nu: 0.29,
    },
    Material {
        key: "ss-304",
        name: "304 stainless, annealed",
        yield_mpa: 215.0,
        ultimate_mpa: 505.0,
        density_g_cm3: 8.00,
        e_gpa: 193.0,
        nu: 0.29,
    },
    Material {
        key: "al-6061-t6",
        name: "Aluminum 6061-T6",
        yield_mpa: 276.0,
        ultimate_mpa: 310.0,
        density_g_cm3: 2.70,
        e_gpa: 68.9,
        nu: 0.33,
    },
    Material {
        key: "al-7075-t6",
        name: "Aluminum 7075-T6",
        yield_mpa: 503.0,
        ultimate_mpa: 572.0,
        density_g_cm3: 2.81,
        e_gpa: 71.7,
        nu: 0.33,
    },
    Material {
        key: "ti-6al4v",
        name: "Titanium 6Al-4V (annealed)",
        yield_mpa: 880.0,
        ultimate_mpa: 950.0,
        density_g_cm3: 4.43,
        e_gpa: 113.8,
        nu: 0.342,
    },
    Material {
        key: "brass-c260-half-hard",
        name: "Brass C26000 (half-hard)",
        yield_mpa: 310.0,
        ultimate_mpa: 425.0,
        density_g_cm3: 8.53,
        e_gpa: 110.0,
        nu: 0.34,
    },
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MaterialArgs {
    /// Material key. Available: `1018-cd`, `4140-qt`, `ss-304`,
    /// `al-6061-t6`, `al-7075-t6`, `ti-6al4v`, `brass-c260-half-hard`.
    material: String,
}

pub struct MachMaterial;
impl Skill for MachMaterial {
    fn name(&self) -> &'static str {
        "mach_material"
    }
    fn description(&self) -> &'static str {
        "Typical mechanical properties for common alloys: yield (MPa), \
        ultimate tensile (MPa), density (g/cm³), Young's modulus (GPa), \
        Poisson's ratio. Values mid-range typical certified for the named \
        condition (cite MatWeb / ASM Handbook). Treat as nominal — always \
        verify with the as-supplied material certificate for design work."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MaterialArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<MaterialArgs>()?;
            let k = a.material.trim().to_ascii_lowercase();
            let m = MATERIALS
                .iter()
                .find(|m| m.key == k)
                .ok_or_else(|| invalid(format!("unknown material '{}'", a.material)))?;
            Ok(text_result(
                json!({
                    "material": m.name,
                    "yield_mpa": m.yield_mpa,
                    "ultimate_mpa": m.ultimate_mpa,
                    "density_g_cm3": m.density_g_cm3,
                    "e_gpa": m.e_gpa,
                    "poisson": m.nu,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "6061-T6 aluminum",
                args: r#"{"material": "al-6061-t6"}"#,
                note: Some("Returns yield, ultimate, density, E, Poisson."),
            },
            SkillExample {
                title: "Annealed 304 stainless",
                args: r#"{"material": "ss-304"}"#,
                note: None,
            },
            SkillExample {
                title: "Ti-6Al-4V",
                args: r#"{"material": "ti-6al4v"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Look up nominal mechanical properties for a common alloy.",
            "Get Young's modulus to plug into beam-deflection or stress-strain calcs.",
            "Estimate part weight from density and volume.",
        ]
    }
}

// ---------------------------------------------------------------------------
// mach_hardness — HRC ↔ HV ↔ HB via ASTM E140 lookup table.
// ---------------------------------------------------------------------------

const HARDNESS_TABLE: &[(f64, f64, f64)] = &[
    // (HRC, HV, HB)
    (20.0, 226.0, 226.0),
    (25.0, 262.0, 256.0),
    (30.0, 302.0, 286.0),
    (35.0, 345.0, 327.0),
    (40.0, 392.0, 371.0),
    (45.0, 446.0, 422.0),
    (50.0, 513.0, 481.0),
    (55.0, 599.0, 560.0),
    (60.0, 697.0, 654.0),
];

fn interp(hardness_table: &[(f64, f64, f64)], hrc: f64) -> Option<(f64, f64)> {
    if hrc < hardness_table[0].0 || hrc > hardness_table[hardness_table.len() - 1].0 {
        return None;
    }
    for w in hardness_table.windows(2) {
        let (h0, v0, b0) = w[0];
        let (h1, v1, b1) = w[1];
        if (h0..=h1).contains(&hrc) {
            let t = (hrc - h0) / (h1 - h0);
            return Some((v0 + t * (v1 - v0), b0 + t * (b1 - b0)));
        }
    }
    None
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HardnessArgs {
    /// Rockwell-C hardness, in the range [20, 60].
    hrc: f64,
}

pub struct MachHardnessConvert;
impl Skill for MachHardnessConvert {
    fn name(&self) -> &'static str {
        "mach_hardness_convert"
    }
    fn description(&self) -> &'static str {
        "Convert Rockwell-C hardness to Vickers (HV) and Brinell (HB), \
        valid HRC 20-60 (non-austenitic steels). Linear interpolation \
        between ASTM E140-12b Table 1 anchor points. Brinell saturates \
        above HB ≈ 650 — values for HRC > 60 are extrapolated approximations \
        and would error here."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HardnessArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<HardnessArgs>()?;
            let (hv, hb) =
                interp(HARDNESS_TABLE, a.hrc).ok_or_else(|| invalid("HRC must be in [20, 60]"))?;
            Ok(text_result(
                json!({ "hrc": a.hrc, "hv": hv, "hb": hb }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Mid-range HRC",
                args: r#"{"hrc": 40}"#,
                note: Some("Returns Vickers (HV) and Brinell (HB) equivalents."),
            },
            SkillExample {
                title: "Soft steel",
                args: r#"{"hrc": 25}"#,
                note: None,
            },
            SkillExample {
                title: "Hardened tool steel",
                args: r#"{"hrc": 58}"#,
                note: Some("Valid range is HRC 20-60."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert Rockwell-C hardness to Vickers or Brinell.",
            "Cross-check a material certificate that lists a different hardness scale.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(MachCuttingSpeed),
        Box::new(MachFeedRate),
        Box::new(MachMrrMilling),
        Box::new(MachCuttingPower),
        Box::new(MachSurfaceFinishTurning),
        Box::new(MachBeamDeflection),
        Box::new(MachSectionInertia),
        Box::new(MachStressStrain),
        Box::new(MachBoltTorque),
        Box::new(MachThreadSpec),
        Box::new(MachMaterial),
        Box::new(MachHardnessConvert),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpm_metric_well_known() {
        // V = 100 m/min on a 20 mm tool → ~1592 rpm.
        let rpm = 1000.0 * 100.0_f64 / (std::f64::consts::PI * 20.0);
        assert!((rpm - 1591.55).abs() < 0.1);
    }

    #[test]
    fn surface_finish_typical() {
        // f = 0.1 mm/rev, r_n = 0.4 mm → Ra ≈ 0.78 μm.
        let ra_mm = 0.1_f64.powi(2) / (32.0 * 0.4);
        assert!((ra_mm * 1000.0 - 0.781).abs() < 0.01);
    }

    #[test]
    fn cantilever_end_deflection_matches_shigley() {
        // δ = PL³/3EI: 100 N, 0.5 m, E=200 GPa, I=1e-9 m⁴ → 0.0208 m.
        let p = 100.0_f64;
        let l = 0.5_f64;
        let e = 200e9_f64;
        let i = 1e-9_f64;
        let d = p * l.powi(3) / (3.0 * e * i);
        assert!((d - 0.02083).abs() < 1.0e-4);
    }

    #[test]
    fn round_section_inertia() {
        // I = π·d⁴/64 for d=0.01 m → 4.909e-10 m⁴.
        let i = std::f64::consts::PI * 0.01_f64.powi(4) / 64.0;
        assert!((i - 4.909e-10).abs() < 1.0e-12);
    }

    #[test]
    fn metric_thread_pitch_minus_p_rule() {
        // ISO metric coarse tap drill ≈ D − P (to within 1 mm half-step).
        for t in THREADS.iter().filter(|t| t.standard == "ISO_M_coarse") {
            let approx = t.major_mm - t.pitch_mm;
            assert!((t.tap_drill_mm - approx).abs() < 0.5, "{}", t.name);
        }
    }

    #[test]
    fn hardness_interp_30_hrc() {
        let (hv, hb) = interp(HARDNESS_TABLE, 30.0).unwrap();
        assert!((hv - 302.0).abs() < 0.1);
        assert!((hb - 286.0).abs() < 0.1);
    }
}
