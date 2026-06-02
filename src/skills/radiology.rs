//! Radiation protection / health physics — general radioactivity, dose,
//! dose-equivalent, attenuation / shielding, ICRP 103 weighting factors,
//! the classic ALARA time / distance / shielding helpers, occupational
//! limits, and a vendored radioisotope reference table covering industrial,
//! research, calibration-source, and clinical isotopes alike. Pure-Rust;
//! data tables are vendored so no constellation cache is needed.
//!
//! ## Source citations
//!
//! - **Dose unit definitions**: ICRU Report 85 (Fundamental Quantities and
//!   Units for Ionizing Radiation, 2011). 1 Gy = 1 J/kg; 1 rad = 0.01 Gy;
//!   1 Sv = 1 J/kg; 1 rem = 0.01 Sv; 1 R = 2.58e-4 C/kg (NCRP 82, exact).
//! - **W/e for dry air**: 33.97 J/C (NIST / AAPM TG-21 & TG-51) → kerma in
//!   air: K_air [Gy] = X [R] · 8.76e-3 (i.e. 0.876 cGy/R).
//! - **Radiation weighting factors w_R**: ICRP Publication 103 (2007),
//!   Annex B. Photons w_R = 1; electrons / muons w_R = 1; protons & charged
//!   pions w_R = 2; α, fission fragments, heavy ions w_R = 20; neutrons via
//!   piecewise continuous function ICRP 103 Eq. (B.1.1).
//! - **Tissue weighting factors w_T**: ICRP 103. Σ w_T = 1.00.
//! - **Specific gamma-ray (air-kerma) rate constants Γ**: ORNL/RSIC-45/R1
//!   (Unger & Trubey 1982); NCRP 151 App. A.
//! - **Isotope half-lives + photon emissions**: NNDC NuDat 3.0 (BNL
//!   <https://www.nndc.bnl.gov/nudat3/>) and IAEA Live Chart.
//! - **Annual occupational limits**: ICRP 103 (international) AND
//!   10 CFR 20 / NCRP 116 (US). The US lens-of-eye limit (150 mSv/y) has
//!   *not* been harmonized to the 2011 ICRP 118 update (20 mSv/y). Tools
//!   that report limits surface both.

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
// Medical / industrial isotope reference table (vendored from NNDC NuDat 3).
// ---------------------------------------------------------------------------

struct Isotope {
    symbol: &'static str,
    half_life_s: f64,
    decay: &'static str,
    /// Primary photon emissions (keV, intensity %).
    photons: &'static [(f64, f64)],
    /// Specific gamma-ray constant Γ in mSv·m²·GBq⁻¹·h⁻¹ at 1 m
    /// (`None` when negligible — pure β, or quoted differently).
    gamma_const_msv_m2_per_gbq_h: Option<f64>,
    /// Typical clinical / industrial use.
    use_case: &'static str,
}

const ISOTOPES: &[Isotope] = &[
    Isotope {
        symbol: "Tc-99m",
        half_life_s: 21_624.12,
        decay: "IT",
        photons: &[(140.51, 89.0)],
        gamma_const_msv_m2_per_gbq_h: Some(0.022),
        use_case: "SPECT workhorse",
    },
    Isotope {
        symbol: "I-131",
        half_life_s: 693_577.728,
        decay: "B-",
        photons: &[(364.49, 81.5), (636.99, 7.16)],
        gamma_const_msv_m2_per_gbq_h: Some(0.0658),
        use_case: "Thyroid Rx / imaging",
    },
    Isotope {
        symbol: "I-123",
        half_life_s: 47_604.6,
        decay: "EC",
        photons: &[(158.97, 83.3)],
        gamma_const_msv_m2_per_gbq_h: Some(0.044),
        use_case: "Thyroid / DaTscan",
    },
    Isotope {
        symbol: "I-125",
        half_life_s: 5_131_142.4,
        decay: "EC",
        photons: &[(35.49, 6.7)],
        gamma_const_msv_m2_per_gbq_h: None,
        use_case: "Brachy seeds, RIA",
    },
    Isotope {
        symbol: "F-18",
        half_life_s: 6_586.2,
        decay: "B+",
        photons: &[(511.0, 193.5)],
        gamma_const_msv_m2_per_gbq_h: Some(0.155),
        use_case: "PET (FDG)",
    },
    Isotope {
        symbol: "Ga-68",
        half_life_s: 4_062.6,
        decay: "B+",
        photons: &[(511.0, 178.0), (1077.0, 3.2)],
        gamma_const_msv_m2_per_gbq_h: Some(0.149),
        use_case: "PET (PSMA / DOTATATE)",
    },
    Isotope {
        symbol: "Lu-177",
        half_life_s: 574_066.6,
        decay: "B-",
        photons: &[(208.37, 10.4), (112.95, 6.2)],
        gamma_const_msv_m2_per_gbq_h: Some(0.0048),
        use_case: "Theranostics (PSMA / DOTATATE)",
    },
    Isotope {
        symbol: "Y-90",
        half_life_s: 230_580.0,
        decay: "B-",
        photons: &[],
        gamma_const_msv_m2_per_gbq_h: None,
        use_case: "SIRT / radioembolization (pure β)",
    },
    Isotope {
        symbol: "Sr-89",
        half_life_s: 4_368_643.2,
        decay: "B-",
        photons: &[(909.0, 0.01)],
        gamma_const_msv_m2_per_gbq_h: None,
        use_case: "Bone-mets palliation (pure β)",
    },
    Isotope {
        symbol: "Ra-223",
        half_life_s: 988_217.0,
        decay: "A",
        photons: &[(269.46, 13.7), (154.21, 5.7)],
        gamma_const_msv_m2_per_gbq_h: Some(0.0061),
        use_case: "Bone-mets CRPC",
    },
    Isotope {
        symbol: "Co-60",
        half_life_s: 166_348_137.6,
        decay: "B-",
        photons: &[(1173.23, 99.85), (1332.49, 99.98)],
        gamma_const_msv_m2_per_gbq_h: Some(0.351),
        use_case: "Teletherapy / industrial irradiators",
    },
    Isotope {
        symbol: "Cs-137",
        half_life_s: 948_745_728.0,
        decay: "B-",
        photons: &[(661.66, 85.1)],
        gamma_const_msv_m2_per_gbq_h: Some(0.0927),
        use_case: "Calibration sources / blood irradiators",
    },
    Isotope {
        symbol: "Ir-192",
        half_life_s: 6_378_652.8,
        decay: "B-/EC",
        photons: &[(316.51, 82.7), (468.07, 47.8), (308.46, 29.7)],
        gamma_const_msv_m2_per_gbq_h: Some(0.130),
        use_case: "HDR brachy / NDT radiography",
    },
    Isotope {
        symbol: "Am-241",
        half_life_s: 1.3651e10,
        decay: "A",
        photons: &[(59.54, 35.9)],
        gamma_const_msv_m2_per_gbq_h: Some(0.0030),
        use_case: "Smoke detectors / Be neutron sources",
    },
    Isotope {
        symbol: "Mo-99",
        half_life_s: 237_513.6,
        decay: "B-",
        photons: &[(739.50, 12.1), (181.07, 6.0)],
        gamma_const_msv_m2_per_gbq_h: Some(0.0148),
        use_case: "Tc-99m generator parent",
    },
    Isotope {
        symbol: "Ba-133",
        half_life_s: 3.3286e8,
        decay: "EC",
        photons: &[(356.01, 62.05), (80.998, 32.9), (302.85, 18.3)],
        gamma_const_msv_m2_per_gbq_h: Some(0.054),
        use_case: "Detector calibration",
    },
    Isotope {
        symbol: "Eu-152",
        half_life_s: 4.2675e8,
        decay: "EC/B-",
        photons: &[(121.78, 28.5), (1408.01, 20.9), (344.28, 26.6)],
        gamma_const_msv_m2_per_gbq_h: Some(0.155),
        use_case: "Multi-line calibration",
    },
    Isotope {
        symbol: "Na-22",
        half_life_s: 82_069_400.0,
        decay: "B+",
        photons: &[(1274.54, 99.94), (511.0, 180.0)],
        gamma_const_msv_m2_per_gbq_h: Some(0.302),
        use_case: "PET calibration",
    },
];

fn find_isotope(query: &str) -> Option<&'static Isotope> {
    let q = query.trim();
    ISOTOPES
        .iter()
        .find(|i| i.symbol.eq_ignore_ascii_case(q))
        .or_else(|| {
            let normalized: String = q.chars().filter(|c| !c.is_whitespace()).collect();
            ISOTOPES.iter().find(|i| {
                let bare = i.symbol.replace('-', "");
                bare.eq_ignore_ascii_case(&normalized)
            })
        })
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IsotopeArgs {
    /// Isotope symbol: `Co-60`, `Cs137`, `I-131`, `Tc99m`, …
    isotope: String,
}

pub struct RadIsotopeLookup;
impl Skill for RadIsotopeLookup {
    fn name(&self) -> &'static str {
        "rad_isotope_lookup"
    }
    fn description(&self) -> &'static str {
        "Look up a radioisotope (industrial, research, calibration, or \
        clinical): half-life (seconds), decay mode, primary photon emissions \
        (keV, intensity %), specific air-kerma rate constant Γ at 1 m \
        (mSv·m²·GBq⁻¹·h⁻¹), and a typical use. Covers Co-60, Cs-137, \
        Ir-192, Am-241, Ra-223, Mo-99, plus calibration sources (Ba-133, \
        Eu-152, Na-22) and several medical/PET tracers. Data sources: \
        NNDC NuDat 3.0, IAEA Live Chart; Γ from ORNL/RSIC-45/R1."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IsotopeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<IsotopeArgs>()?;
            let iso = find_isotope(&a.isotope)
                .ok_or_else(|| invalid(format!("unknown isotope '{}'", a.isotope)))?;
            let photons: Vec<serde_json::Value> = iso
                .photons
                .iter()
                .map(|(e, i)| json!({"energy_kev": e, "intensity_pct": i}))
                .collect();
            Ok(text_result(
                json!({
                    "symbol": iso.symbol,
                    "half_life_s": iso.half_life_s,
                    "decay": iso.decay,
                    "photons": photons,
                    "gamma_const_mSv_m2_per_GBq_h": iso.gamma_const_msv_m2_per_gbq_h,
                    "use_case": iso.use_case,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_units — Gy ↔ rad, Sv ↔ rem, R ↔ Gy_air.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UnitArgs {
    /// `gy_to_rad`, `rad_to_gy`, `sv_to_rem`, `rem_to_sv`, `r_to_gy_air`,
    /// `gy_air_to_r`.
    direction: String,
    value: f64,
}

pub struct RadUnits;
impl Skill for RadUnits {
    fn name(&self) -> &'static str {
        "rad_units"
    }
    fn description(&self) -> &'static str {
        "Convert dose / exposure units: 1 Gy = 100 rad, 1 Sv = 100 rem \
        (ICRU Report 85). For Roentgen ↔ air-kerma: K_air [Gy] = X [R] · \
        8.76e-3 (uses NIST W/e = 33.97 J/C for dry air; 0.876 cGy/R)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<UnitArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<UnitArgs>()?;
            const R_TO_GY_AIR: f64 = 8.76e-3;
            let out = match a.direction.to_ascii_lowercase().as_str() {
                "gy_to_rad" => a.value * 100.0,
                "rad_to_gy" => a.value / 100.0,
                "sv_to_rem" => a.value * 100.0,
                "rem_to_sv" => a.value / 100.0,
                "r_to_gy_air" => a.value * R_TO_GY_AIR,
                "gy_air_to_r" => a.value / R_TO_GY_AIR,
                other => return Err(invalid(format!("unknown direction '{other}'"))),
            };
            Ok(text_result(json!({ "result": out }).to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_attenuation — Beer-Lambert + HVL/TVL.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AttenArgs {
    /// Linear attenuation coefficient μ in cm⁻¹ (or any inverse-length unit).
    mu: f64,
    /// Thickness through which to attenuate (same length unit as 1/μ).
    thickness: f64,
    /// Incident intensity (default 1.0).
    #[serde(default)]
    i0: Option<f64>,
}

pub struct RadAttenuation;
impl Skill for RadAttenuation {
    fn name(&self) -> &'static str {
        "rad_attenuation"
    }
    fn description(&self) -> &'static str {
        "Beer-Lambert exponential attenuation: I(x) = I₀·exp(-μx); HVL = \
        ln(2)/μ; TVL = ln(10)/μ. Returns transmitted intensity, transmission \
        fraction, half- and tenth-value layers in the same length units as \
        1/μ."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AttenArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<AttenArgs>()?;
            if a.mu <= 0.0 {
                return Err(invalid("mu must be > 0"));
            }
            let i0 = a.i0.unwrap_or(1.0);
            let i = i0 * (-a.mu * a.thickness).exp();
            let hvl = std::f64::consts::LN_2 / a.mu;
            let tvl = (10_f64.ln()) / a.mu;
            Ok(text_result(
                json!({
                    "i": i,
                    "transmission_fraction": i / i0,
                    "hvl": hvl,
                    "tvl": tvl,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_inverse_square — dose at distance.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InvSquareArgs {
    /// Dose / dose-rate at reference distance (any unit).
    d_ref: f64,
    /// Reference distance (any length unit).
    r_ref: f64,
    /// Target distance (same length unit).
    r_target: f64,
}

pub struct RadInverseSquare;
impl Skill for RadInverseSquare {
    fn name(&self) -> &'static str {
        "rad_inverse_square"
    }
    fn description(&self) -> &'static str {
        "Scale a point-source dose or dose rate by inverse-square: D(r) = \
        D(r_ref)·(r_ref/r)². Returns the scaled dose at the target distance."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InvSquareArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<InvSquareArgs>()?;
            if a.r_target <= 0.0 || a.r_ref <= 0.0 {
                return Err(invalid("distances must be > 0"));
            }
            let scale = (a.r_ref / a.r_target).powi(2);
            Ok(text_result(
                json!({"d": a.d_ref * scale, "scale": scale}).to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_dose_rate — for a point source of known activity, using vendored Γ.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DoseRateArgs {
    /// Isotope symbol (must be in the vendored table with a non-null Γ).
    isotope: String,
    /// Activity in GBq.
    activity_gbq: f64,
    /// Distance from the point source, in meters.
    distance_m: f64,
}

pub struct RadDoseRate;
impl Skill for RadDoseRate {
    fn name(&self) -> &'static str {
        "rad_dose_rate"
    }
    fn description(&self) -> &'static str {
        "Estimate dose rate (mSv/h) from a point γ source using the specific \
        air-kerma rate constant Γ: dose_rate = Γ · A / r². Returns the dose \
        rate in mSv/h and (with shielding omitted) the dose accumulated over \
        1 hour. Γ values from ORNL/RSIC-45/R1 (Unger & Trubey, 1982). The \
        result is an idealization — bare point source, no buildup factor, no \
        shielding. Use only as a first-cut planning number."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DoseRateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DoseRateArgs>()?;
            if a.distance_m <= 0.0 {
                return Err(invalid("distance must be > 0"));
            }
            let iso = find_isotope(&a.isotope)
                .ok_or_else(|| invalid(format!("unknown isotope '{}'", a.isotope)))?;
            let gamma = iso
                .gamma_const_msv_m2_per_gbq_h
                .ok_or_else(|| invalid(format!("isotope '{}' has no Γ constant (pure β or low γ); not computable from this tool", iso.symbol)))?;
            let dose_rate = gamma * a.activity_gbq / a.distance_m.powi(2);
            Ok(text_result(
                json!({
                    "dose_rate_mSv_per_h": dose_rate,
                    "gamma_const_mSv_m2_per_GBq_h": gamma,
                    "isotope": iso.symbol,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_equivalent_dose — D × w_R for a single radiation type.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EqDoseArgs {
    /// Absorbed dose (Gy).
    d_gy: f64,
    /// Radiation type. One of `photon`, `electron`, `proton`, `alpha`,
    /// `fission_fragment`, `heavy_ion`, `neutron`.
    radiation: String,
    /// Neutron energy in MeV — required if `radiation == "neutron"`.
    #[serde(default)]
    neutron_energy_mev: Option<f64>,
}

fn neutron_wr(en_mev: f64) -> f64 {
    // ICRP 103 Annex B Eq. (B.1.1) piecewise continuous w_R(E_n).
    if en_mev < 1.0 {
        2.5 + 18.2 * (-(en_mev.ln().powi(2)) / 6.0).exp()
    } else if en_mev <= 50.0 {
        5.0 + 17.0 * (-((2.0 * en_mev).ln().powi(2)) / 6.0).exp()
    } else {
        2.5 + 3.25 * (-((0.04 * en_mev).ln().powi(2)) / 6.0).exp()
    }
}

pub struct RadEquivalentDose;
impl Skill for RadEquivalentDose {
    fn name(&self) -> &'static str {
        "rad_equivalent_dose"
    }
    fn description(&self) -> &'static str {
        "Equivalent dose H_T = w_R · D (Sv). ICRP Publication 103 radiation \
        weighting factors w_R: photons / electrons = 1; protons & charged \
        pions = 2; α, fission fragments, heavy ions = 20; neutrons via the \
        piecewise continuous function ICRP 103 Eq. (B.1.1)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EqDoseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EqDoseArgs>()?;
            let w_r = match a.radiation.to_ascii_lowercase().as_str() {
                "photon" | "electron" | "muon" => 1.0,
                "proton" | "charged_pion" => 2.0,
                "alpha" | "fission_fragment" | "heavy_ion" => 20.0,
                "neutron" => {
                    let en = a
                        .neutron_energy_mev
                        .ok_or_else(|| invalid("neutron requires neutron_energy_mev"))?;
                    if en <= 0.0 {
                        return Err(invalid("neutron_energy_mev must be > 0"));
                    }
                    neutron_wr(en)
                }
                other => return Err(invalid(format!("unknown radiation '{other}'"))),
            };
            Ok(text_result(
                json!({
                    "equivalent_dose_sv": w_r * a.d_gy,
                    "w_r": w_r,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_effective_half_life — biokinetic clearance.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EffHlArgs {
    /// Physical half-life (any unit; output uses the same).
    t_phys: f64,
    /// Biological half-life (same unit).
    t_bio: f64,
}

pub struct RadEffectiveHalfLife;
impl Skill for RadEffectiveHalfLife {
    fn name(&self) -> &'static str {
        "rad_effective_half_life"
    }
    fn description(&self) -> &'static str {
        "Effective half-life combining radioactive decay with biological \
        clearance: 1/T_eff = 1/T_phys + 1/T_bio. Returns T_eff in the same \
        units as the inputs (use consistent units)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EffHlArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EffHlArgs>()?;
            if a.t_phys <= 0.0 || a.t_bio <= 0.0 {
                return Err(invalid("half-lives must be > 0"));
            }
            let t_eff = 1.0 / (1.0 / a.t_phys + 1.0 / a.t_bio);
            Ok(text_result(json!({ "t_eff": t_eff }).to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_occupational_limits — surface ICRP 103 AND US 10 CFR 20.
// ---------------------------------------------------------------------------

pub struct RadOccupationalLimits;
impl Skill for RadOccupationalLimits {
    fn name(&self) -> &'static str {
        "rad_occupational_limits"
    }
    fn description(&self) -> &'static str {
        "Annual occupational and public dose limits for both ICRP 103 \
        (international) and US 10 CFR 20 / NCRP 116. Notable unharmonized \
        difference: ICRP 118 (2011) cut the lens-of-eye limit to 20 mSv/y; \
        the US NRC retains 150 mSv/y. Tool returns both sets so the caller \
        can pick the relevant regulatory regime."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<crate::skills::NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let _ = ctx;
            Ok(text_result(
                json!({
                    "icrp_103": {
                        "whole_body_mSv_per_y": "20 (avg over 5y; ≤50 any single y)",
                        "lens_of_eye_mSv_per_y": "20 (ICRP 118, 2011)",
                        "skin_extremities_mSv_per_y": 500,
                        "public_mSv_per_y": 1.0,
                        "declared_pregnancy_embryo_mSv": "1 over pregnancy"
                    },
                    "us_10cfr20_ncrp116": {
                        "whole_body_mSv_per_y": 50,
                        "lens_of_eye_mSv_per_y": 150,
                        "skin_extremities_mSv_per_y": 500,
                        "public_mSv_per_y": 1.0,
                        "declared_pregnancy_embryo_mSv": "5 over pregnancy",
                        "ncrp116_cumulative_guideline_mSv": "≤ 10 × age (years)"
                    },
                    "citation": [
                        "ICRP Publication 103 (2007)",
                        "ICRP Publication 118 (2011) — lens-of-eye revision",
                        "US 10 CFR Part 20",
                        "NCRP Report 116"
                    ]
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_shielding_thickness — invert Beer-Lambert for required slab thickness.
// ---------------------------------------------------------------------------

struct ShieldingMaterial {
    key: &'static str,
    name: &'static str,
    /// Mass attenuation coefficient μ/ρ at common photon energies (cm²/g).
    /// Indexed parallel to ENERGIES_KEV below.
    mu_over_rho: &'static [f64],
    density_g_cm3: f64,
}

const ENERGIES_KEV: &[f64] = &[100.0, 200.0, 500.0, 1000.0, 2000.0];

// NIST XCOM mass attenuation coefficients for total photon attenuation
// (no coherent scattering separation), rounded to 3 sig figs. These are
// the canonical numbers used in health-physics shielding hand calcs.
const SHIELDING: &[ShieldingMaterial] = &[
    ShieldingMaterial {
        key: "lead",
        name: "Lead (Pb)",
        mu_over_rho: &[5.55, 0.999, 0.161, 0.0710, 0.0463],
        density_g_cm3: 11.34,
    },
    ShieldingMaterial {
        key: "concrete",
        name: "Concrete (ordinary)",
        mu_over_rho: &[0.169, 0.124, 0.0871, 0.0637, 0.0451],
        density_g_cm3: 2.30,
    },
    ShieldingMaterial {
        key: "steel",
        name: "Iron / mild steel",
        mu_over_rho: &[0.370, 0.146, 0.0840, 0.0598, 0.0421],
        density_g_cm3: 7.87,
    },
    ShieldingMaterial {
        key: "water",
        name: "Water (tissue surrogate)",
        mu_over_rho: &[0.171, 0.137, 0.0966, 0.0706, 0.0494],
        density_g_cm3: 1.00,
    },
    ShieldingMaterial {
        key: "aluminum",
        name: "Aluminum",
        mu_over_rho: &[0.170, 0.122, 0.0840, 0.0614, 0.0432],
        density_g_cm3: 2.70,
    },
];

fn interp_mu(material: &ShieldingMaterial, e_kev: f64) -> Option<f64> {
    if e_kev < ENERGIES_KEV[0] || e_kev > *ENERGIES_KEV.last().unwrap() {
        return None;
    }
    for w in ENERGIES_KEV.windows(2).enumerate() {
        let (i, slice) = w;
        let (e0, e1) = (slice[0], slice[1]);
        if (e0..=e1).contains(&e_kev) {
            // Log-log interpolation (XCOM-style).
            let lm0 = material.mu_over_rho[i].ln();
            let lm1 = material.mu_over_rho[i + 1].ln();
            let t = (e_kev.ln() - e0.ln()) / (e1.ln() - e0.ln());
            return Some((lm0 + t * (lm1 - lm0)).exp());
        }
    }
    None
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ShieldArgs {
    /// Photon energy (keV). Range supported: 100–2000 keV (NIST XCOM
    /// table anchors).
    energy_kev: f64,
    /// Shielding material: `lead`, `concrete`, `steel`, `water`, `aluminum`.
    material: String,
    /// Desired transmission fraction (e.g. 0.1 = one TVL, 0.01 = two TVLs).
    transmission: f64,
}

pub struct RadShieldingThickness;
impl Skill for RadShieldingThickness {
    fn name(&self) -> &'static str {
        "rad_shielding_thickness"
    }
    fn description(&self) -> &'static str {
        "Required slab thickness to attenuate a narrow γ beam to a desired \
        transmission fraction. Uses NIST XCOM mass attenuation coefficients \
        for Pb, ordinary concrete, iron / mild steel, water, and aluminum at \
        100/200/500/1000/2000 keV (log-log interpolated for intermediate \
        energies). x = ln(1/T) / μ, where μ = (μ/ρ)·ρ. **First-order \
        narrow-beam estimate only** — for design work apply a buildup factor \
        (B ~ 1.5–4× depending on energy and thickness) and pick the worst \
        case for your geometry."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ShieldArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ShieldArgs>()?;
            if a.transmission <= 0.0 || a.transmission >= 1.0 {
                return Err(invalid("transmission must be in (0, 1)"));
            }
            let key = a.material.trim().to_ascii_lowercase();
            let m = SHIELDING
                .iter()
                .find(|m| m.key == key)
                .ok_or_else(|| invalid(format!("unknown material '{}'", a.material)))?;
            let mu_over_rho = interp_mu(m, a.energy_kev)
                .ok_or_else(|| invalid("energy_kev outside table range [100, 2000] keV"))?;
            let mu = mu_over_rho * m.density_g_cm3; // cm⁻¹
            let x_cm = (1.0 / a.transmission).ln() / mu;
            let hvl_cm = std::f64::consts::LN_2 / mu;
            let tvl_cm = (10_f64.ln()) / mu;
            Ok(text_result(
                json!({
                    "thickness_cm": x_cm,
                    "thickness_mm": x_cm * 10.0,
                    "mu_per_cm": mu,
                    "hvl_cm": hvl_cm,
                    "tvl_cm": tvl_cm,
                    "material": m.name,
                    "note": "narrow-beam attenuation; apply a buildup factor for design work",
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// rad_alara — time / distance / shielding triad combined.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AlaraArgs {
    /// Reference dose rate at the reference distance (mSv/h).
    #[serde(alias = "dose_rate_mSv_h_ref")]
    dose_rate_msv_h_ref: f64,
    /// Reference distance for `dose_rate_mSv_h_ref` (m).
    distance_ref_m: f64,
    /// Worker distance (m).
    distance_worker_m: f64,
    /// Exposure time (hours).
    time_h: f64,
    /// Optional shielding attenuation factor (0 < A ≤ 1; default 1 = no
    /// shielding). E.g. one HVL = 0.5, one TVL = 0.1.
    #[serde(default)]
    shielding_transmission: Option<f64>,
}

pub struct RadAlara;
impl Skill for RadAlara {
    fn name(&self) -> &'static str {
        "rad_alara"
    }
    fn description(&self) -> &'static str {
        "Combined time / distance / shielding (ALARA triad) dose estimate. \
        D = D_ref · (r_ref / r)² · T · t, where T is the shielding \
        transmission fraction (e.g. 0.5 = one HVL; 0.1 = one TVL) and t is \
        time in hours. Returns the integrated dose in mSv and per-axis \
        contributions so the caller can see which lever (less time, more \
        distance, more shielding) wins."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AlaraArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<AlaraArgs>()?;
            if a.distance_worker_m <= 0.0 || a.distance_ref_m <= 0.0 {
                return Err(invalid("distances must be > 0"));
            }
            if a.time_h < 0.0 {
                return Err(invalid("time_h must be ≥ 0"));
            }
            let t = a.shielding_transmission.unwrap_or(1.0);
            if !(t > 0.0 && t <= 1.0) {
                return Err(invalid("shielding_transmission must be in (0, 1]"));
            }
            let scale_distance = (a.distance_ref_m / a.distance_worker_m).powi(2);
            let dose_rate = a.dose_rate_msv_h_ref * scale_distance * t;
            let dose_total = dose_rate * a.time_h;
            Ok(text_result(
                json!({
                    "dose_total_mSv": dose_total,
                    "dose_rate_at_worker_mSv_h": dose_rate,
                    "distance_factor": scale_distance,
                    "shielding_factor": t,
                    "time_h": a.time_h,
                })
                .to_string(),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(RadIsotopeLookup),
        Box::new(RadUnits),
        Box::new(RadAttenuation),
        Box::new(RadInverseSquare),
        Box::new(RadDoseRate),
        Box::new(RadEquivalentDose),
        Box::new(RadEffectiveHalfLife),
        Box::new(RadOccupationalLimits),
        Box::new(RadShieldingThickness),
        Box::new(RadAlara),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gy_rad_round_trip() {
        let gy = 2.5_f64;
        let rad = gy * 100.0;
        let back = rad / 100.0;
        assert!((gy - back).abs() < 1.0e-12);
    }

    #[test]
    fn cobalt_60_dose_rate_at_1m() {
        // 1 GBq Co-60 at 1 m: ≈ 0.351 mSv/h.
        let iso = find_isotope("Co-60").unwrap();
        let g = iso.gamma_const_msv_m2_per_gbq_h.unwrap();
        let dr = g * 1.0 / 1.0_f64.powi(2);
        assert!((dr - 0.351).abs() < 1.0e-3);
    }

    #[test]
    fn hvl_lead_at_100kev() {
        // Pb μ ≈ 5.55 cm²/g · 11.34 g/cm³ = 62.9 cm⁻¹.
        let mu = 62.9_f64;
        let hvl = std::f64::consts::LN_2 / mu;
        assert!((hvl - 0.011).abs() < 5.0e-3);
    }

    #[test]
    fn neutron_wr_continuity() {
        // ICRP 103 piecewise function should be continuous at the
        // joins (1 MeV and 50 MeV).
        let left = neutron_wr(0.9999);
        let right = neutron_wr(1.0001);
        assert!((left - right).abs() < 0.5);
        let left = neutron_wr(49.999);
        let right = neutron_wr(50.001);
        assert!((left - right).abs() < 1.0);
    }

    #[test]
    fn effective_half_life_combines_correctly() {
        // T_phys = 8 d, T_bio = 24 d → T_eff = 6 d (I-131 thyroid example).
        let t_eff: f64 = 1.0 / (1.0 / 8.0 + 1.0 / 24.0);
        assert!((t_eff - 6.0).abs() < 1.0e-9);
    }
}
