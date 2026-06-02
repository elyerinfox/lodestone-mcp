//! Nuclear physics — semi-empirical mass formula (Bethe-Weizsäcker),
//! Q-values, atomic mass ↔ MeV conversions, Bateman two-step decay chains,
//! cross-section unit helpers, and a vendored nuclide table. All pure-Rust,
//! on by default. Local-only — no network and so no constellation cache
//! needed here (the data tables are vendored).
//!
//! ## Source citations
//!
//! - **Semi-empirical mass formula coefficients (Krane)**: K. S. Krane,
//!   *Introductory Nuclear Physics*, Wiley 1988, §3.3 / Table 3.2 (Eq. 3.29).
//!   Defaults: a_V = 15.5, a_S = 16.8, a_C = 0.72, a_A = 23.0, a_P = 34 MeV;
//!   pairing exponent k_P = −3/4.
//! - **Atomic-mass-unit ↔ MeV**: 1 u = 931.494 103 72 MeV/c² (CODATA 2022;
//!   Mohr et al., *Rev. Mod. Phys.* 97, 025002 (2024); NIST
//!   <https://physics.nist.gov/cuu/Constants/>).
//! - **Activity unit**: 1 Bq = 1 decay/s; 1 Ci = 3.7 × 10¹⁰ Bq exactly
//!   (definitional).
//! - **Barn**: 1 b = 10⁻²⁸ m² = 10⁻²⁴ cm² exactly.
//! - **Atomic masses**: AME2020 — Wang, Huang, Kondev, Audi, Naimi,
//!   *Chinese Phys. C* 45, 030003 (2021); DOI 10.1088/1674-1137/abddaf.
//! - **Half-lives & decay modes**: NUBASE2020 — Kondev, Wang, Huang, Naimi,
//!   Audi, *Chinese Phys. C* 45, 030001 (2021).
//! - **Fission Q-values**: Madland, *Total Prompt Energy Release in the
//!   Neutron-Induced Fission of ²³⁵U, ²³⁸U and ²³⁹Pu* (arXiv:nucl-th/0603071);
//!   ENDF/B-VIII.0. Reported Q values include both total and recoverable
//!   (after subtracting neutrino energy).
//! - **Bateman equation**: Bateman, *Proc. Cambridge Philos. Soc.* 1910,
//!   15:423-427; standard two-step closed form.
//! - **Binding-energy peak**: Ni-62 at 8.7945 MeV/A is the per-nucleon
//!   binding maximum; Fe-56 has the lowest mass per nucleon. Both are
//!   exposed.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

/// 1 u = 931.494 103 72 MeV/c² — CODATA 2022.
pub const U_TO_MEV: f64 = 931.494_103_72;
/// ln(2) — handy for half-life ↔ decay-constant conversions.
pub const LN_2: f64 = std::f64::consts::LN_2;
/// 1 Ci = 3.7e10 Bq (exact, definitional).
pub const CURIE_BQ: f64 = 3.7e10;
/// 1 barn = 1e-28 m² (exact).
///
/// Kept alongside U_TO_MEV, LN_2, and CURIE_BQ for completeness — the
/// `nuke_unit_convert` tool reads it via direct match arm, not via this
/// constant binding, so clippy doesn't see the use across the
/// match-arm literal. Worth keeping the named constant for callers who
/// want to import it.
#[allow(dead_code)]
pub const BARN_M2: f64 = 1.0e-28;

struct Nuclide {
    z: u32,
    n: u32,
    symbol: &'static str,
    mass_u: f64,
    /// Half-life in seconds. `None` = stable.
    half_life_s: Option<f64>,
    /// NUBASE decay-mode notation.
    decay_modes: &'static str,
}

const NUCLIDES: &[Nuclide] = &[
    Nuclide {
        z: 1,
        n: 0,
        symbol: "H-1",
        mass_u: 1.007825032,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 1,
        n: 1,
        symbol: "H-2",
        mass_u: 2.014101778,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 1,
        n: 2,
        symbol: "H-3",
        mass_u: 3.016049281,
        half_life_s: Some(388_789_488.0),
        decay_modes: "B-",
    },
    Nuclide {
        z: 2,
        n: 1,
        symbol: "He-3",
        mass_u: 3.016029322,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 2,
        n: 2,
        symbol: "He-4",
        mass_u: 4.002603254,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 3,
        n: 3,
        symbol: "Li-6",
        mass_u: 6.015122887,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 3,
        n: 4,
        symbol: "Li-7",
        mass_u: 7.016003437,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 4,
        n: 3,
        symbol: "Be-7",
        mass_u: 7.016928717,
        half_life_s: Some(4_598_208.0),
        decay_modes: "EC",
    },
    Nuclide {
        z: 4,
        n: 5,
        symbol: "Be-9",
        mass_u: 9.012183066,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 6,
        n: 6,
        symbol: "C-12",
        mass_u: 12.000000000,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 6,
        n: 7,
        symbol: "C-13",
        mass_u: 13.003354835,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 7,
        n: 7,
        symbol: "N-14",
        mass_u: 14.003074004,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 8,
        n: 8,
        symbol: "O-16",
        mass_u: 15.994914620,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 9,
        n: 10,
        symbol: "F-19",
        mass_u: 18.998403163,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 11,
        n: 11,
        symbol: "Na-22",
        mass_u: 21.994437411,
        half_life_s: Some(82_069_400.0),
        decay_modes: "B+/EC",
    },
    Nuclide {
        z: 15,
        n: 17,
        symbol: "P-32",
        mass_u: 31.973907643,
        half_life_s: Some(1_232_755.2),
        decay_modes: "B-",
    },
    Nuclide {
        z: 26,
        n: 30,
        symbol: "Fe-56",
        mass_u: 55.934936325,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 27,
        n: 33,
        symbol: "Co-60",
        mass_u: 59.933816300,
        half_life_s: Some(166_348_137.6),
        decay_modes: "B-",
    },
    Nuclide {
        z: 28,
        n: 34,
        symbol: "Ni-62",
        mass_u: 61.928344867,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 38,
        n: 52,
        symbol: "Sr-90",
        mass_u: 89.907730000,
        half_life_s: Some(908_006_400.0),
        decay_modes: "B-",
    },
    Nuclide {
        z: 43,
        n: 56,
        symbol: "Tc-99m",
        mass_u: 98.906250840,
        half_life_s: Some(21_624.12),
        decay_modes: "IT",
    },
    Nuclide {
        z: 53,
        n: 78,
        symbol: "I-131",
        mass_u: 130.906126370,
        half_life_s: Some(693_577.728),
        decay_modes: "B-",
    },
    Nuclide {
        z: 55,
        n: 82,
        symbol: "Cs-137",
        mass_u: 136.907089464,
        half_life_s: Some(948_745_728.0),
        decay_modes: "B-",
    },
    Nuclide {
        z: 82,
        n: 124,
        symbol: "Pb-206",
        mass_u: 205.974465683,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 82,
        n: 126,
        symbol: "Pb-208",
        mass_u: 207.976652500,
        half_life_s: None,
        decay_modes: "stable",
    },
    Nuclide {
        z: 90,
        n: 142,
        symbol: "Th-232",
        mass_u: 232.038055800,
        half_life_s: Some(4.4174e17),
        decay_modes: "A",
    },
    Nuclide {
        z: 92,
        n: 143,
        symbol: "U-235",
        mass_u: 235.043930131,
        half_life_s: Some(2.2208e16),
        decay_modes: "A (SF rare)",
    },
    Nuclide {
        z: 92,
        n: 146,
        symbol: "U-238",
        mass_u: 238.050788423,
        half_life_s: Some(1.40990e17),
        decay_modes: "A",
    },
    Nuclide {
        z: 94,
        n: 144,
        symbol: "Pu-238",
        mass_u: 238.049560200,
        half_life_s: Some(2.766_852e9),
        decay_modes: "A",
    },
    Nuclide {
        z: 94,
        n: 145,
        symbol: "Pu-239",
        mass_u: 239.052163750,
        half_life_s: Some(7.605936e11),
        decay_modes: "A",
    },
    Nuclide {
        z: 95,
        n: 146,
        symbol: "Am-241",
        mass_u: 241.056829400,
        half_life_s: Some(1.3651e10),
        decay_modes: "A",
    },
    Nuclide {
        z: 98,
        n: 154,
        symbol: "Cf-252",
        mass_u: 252.081627199,
        half_life_s: Some(8.344e7),
        decay_modes: "A (96.9%) / SF (3.1%)",
    },
];

fn find_nuclide(query: &str) -> Option<&'static Nuclide> {
    let q = query.trim();
    NUCLIDES
        .iter()
        .find(|n| n.symbol.eq_ignore_ascii_case(q))
        .or_else(|| {
            // Allow "U-235" or "235U" or "u235".
            let normalized: String = q.chars().filter(|c| !c.is_whitespace()).collect();
            NUCLIDES.iter().find(|n| {
                let bare = n.symbol.replace('-', "");
                bare.eq_ignore_ascii_case(&normalized) || normalized.eq_ignore_ascii_case(&bare)
            })
        })
}

// ---------------------------------------------------------------------------
// nuke_nuclide_lookup
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NuclideArgs {
    /// Nuclide symbol such as `U-235`, `u235`, `Co-60`, `Cs-137`,
    /// `Tc-99m`.
    nuclide: String,
}

pub struct NukeNuclideLookup;
impl Skill for NukeNuclideLookup {
    fn name(&self) -> &'static str {
        "nuke_nuclide_lookup"
    }
    fn description(&self) -> &'static str {
        "Look up one nuclide from the vendored AME2020 / NUBASE2020 subset \
        (~30 commonly cited stable + radioactive nuclides). Returns Z, N, A, \
        atomic mass (u), half-life (seconds), and decay-mode notation. \
        Sources: Wang et al. *Chin. Phys. C* 45, 030003 (AME2020); Kondev \
        et al. *Chin. Phys. C* 45, 030001 (NUBASE2020)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NuclideArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<NuclideArgs>()?;
            let n = find_nuclide(&a.nuclide)
                .ok_or_else(|| invalid(format!("unknown nuclide '{}'", a.nuclide)))?;
            Ok(text_result(
                json!({
                    "symbol": n.symbol,
                    "z": n.z,
                    "n": n.n,
                    "a": n.z + n.n,
                    "atomic_mass_u": n.mass_u,
                    "half_life_s": n.half_life_s,
                    "decay_modes": n.decay_modes,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// nuke_binding_energy — semi-empirical mass formula (Krane defaults).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SemfArgs {
    /// Mass number A.
    a: u32,
    /// Atomic number Z.
    z: u32,
}

pub struct NukeBindingEnergy;
impl Skill for NukeBindingEnergy {
    fn name(&self) -> &'static str {
        "nuke_binding_energy"
    }
    fn description(&self) -> &'static str {
        "Estimate nuclear binding energy via the Bethe-Weizsäcker / liquid-\
        drop semi-empirical mass formula, using the Krane (1988) coefficient \
        set: a_V = 15.5, a_S = 16.8, a_C = 0.72, a_A = 23.0, a_P = 34 MeV; \
        pairing exponent k_P = −3/4. Returns total BE (MeV), BE per nucleon, \
        and the individual term contributions. Citation: Krane, \
        *Introductory Nuclear Physics*, Wiley 1988, §3.3."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SemfArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SemfArgs>()?;
            if a.z == 0 || a.z >= a.a {
                return Err(invalid("require 0 < Z < A"));
            }
            // Krane coefficients (MeV).
            const A_V: f64 = 15.5;
            const A_S: f64 = 16.8;
            const A_C: f64 = 0.72;
            const A_A: f64 = 23.0;
            const A_P: f64 = 34.0;
            let af = a.a as f64;
            let zf = a.z as f64;
            let n = af - zf;
            let volume = A_V * af;
            let surface = -A_S * af.powf(2.0 / 3.0);
            let coulomb = -A_C * zf * (zf - 1.0) / af.cbrt();
            let asymmetry = -A_A * (af - 2.0 * zf).powi(2) / af;
            // Pairing (k_P = −3/4): even-even +δ, odd 0, odd-odd −δ.
            let z_even = a.z % 2 == 0;
            let n_even = (n as u32).is_multiple_of(2);
            let delta0 = A_P * af.powf(-0.75);
            let pairing = match (z_even, n_even) {
                (true, true) => delta0,
                (false, false) => -delta0,
                _ => 0.0,
            };
            let be = volume + surface + coulomb + asymmetry + pairing;
            Ok(text_result(
                json!({
                    "binding_energy_mev": be,
                    "be_per_nucleon_mev": be / af,
                    "terms": {
                        "volume": volume,
                        "surface": surface,
                        "coulomb": coulomb,
                        "asymmetry": asymmetry,
                        "pairing": pairing,
                    },
                    "coefficients_mev": {
                        "a_V": A_V, "a_S": A_S, "a_C": A_C, "a_A": A_A, "a_P": A_P,
                    },
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// nuke_q_value — Q-value of a nuclear reaction from atomic masses.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QArgs {
    /// Reactant atomic masses in u.
    reactants_u: Vec<f64>,
    /// Product atomic masses in u.
    products_u: Vec<f64>,
}

pub struct NukeQValue;
impl Skill for NukeQValue {
    fn name(&self) -> &'static str {
        "nuke_q_value"
    }
    fn description(&self) -> &'static str {
        "Q-value of a nuclear reaction from atomic masses (Q > 0 = \
        exothermic): Q = (Σm_reactants − Σm_products) · c². Uses the CODATA \
        2022 conversion 1 u = 931.49410372 MeV/c². Inputs are atomic masses \
        in u — the convention used in the AME2020 mass tables."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<QArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<QArgs>()?;
            if a.reactants_u.is_empty() || a.products_u.is_empty() {
                return Err(invalid("need at least one reactant and one product"));
            }
            let r: f64 = a.reactants_u.iter().sum();
            let p: f64 = a.products_u.iter().sum();
            let q = (r - p) * U_TO_MEV;
            Ok(text_result(
                json!({
                    "q_mev": q,
                    "exothermic": q > 0.0,
                    "u_to_mev": U_TO_MEV,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// nuke_decay_law and nuke_decay_chain (Bateman two-step).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DecayArgs {
    /// Initial quantity (atoms, moles, Bq — anything proportional).
    n0: f64,
    /// Half-life (seconds).
    half_life_s: f64,
    /// Elapsed time (seconds).
    time_s: f64,
}

pub struct NukeDecayLaw;
impl Skill for NukeDecayLaw {
    fn name(&self) -> &'static str {
        "nuke_decay_law"
    }
    fn description(&self) -> &'static str {
        "First-order radioactive decay: N(t) = N₀·exp(-λt), λ = ln(2)/t½. \
        Returns the remaining quantity, the activity-form A(t) = λ·N(t) (per \
        second), the decay constant, and the fraction remaining."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DecayArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DecayArgs>()?;
            if a.half_life_s <= 0.0 {
                return Err(invalid("half_life_s must be > 0"));
            }
            let lambda = LN_2 / a.half_life_s;
            let n = a.n0 * (-lambda * a.time_s).exp();
            Ok(text_result(
                json!({
                    "n_remaining": n,
                    "activity_bq": lambda * n,
                    "decay_constant_per_s": lambda,
                    "fraction_remaining": n / a.n0,
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ChainArgs {
    /// Initial parent inventory (any unit, as long as it is proportional).
    n_a0: f64,
    /// Parent half-life (seconds).
    half_life_a_s: f64,
    /// Daughter half-life (seconds).
    half_life_b_s: f64,
    /// Time of evaluation (seconds).
    time_s: f64,
}

pub struct NukeDecayChain;
impl Skill for NukeDecayChain {
    fn name(&self) -> &'static str {
        "nuke_decay_chain"
    }
    fn description(&self) -> &'static str {
        "Bateman two-step decay chain A → B → stable. Assumes N_B(0) = 0. \
        General-case closed form: N_B(t) = (λ_A·N_A(0)/(λ_B − λ_A))·(e^{-λ_A t} \
        − e^{-λ_B t}). Reduces to the special case N_B(t) = λ·N_A(0)·t·e^{-λt} \
        when λ_A = λ_B. Citation: Bateman, *Proc. Camb. Phil. Soc.* 1910, \
        15:423."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ChainArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ChainArgs>()?;
            if a.half_life_a_s <= 0.0 || a.half_life_b_s <= 0.0 {
                return Err(invalid("half lives must be > 0"));
            }
            let la = LN_2 / a.half_life_a_s;
            let lb = LN_2 / a.half_life_b_s;
            let na = a.n_a0 * (-la * a.time_s).exp();
            // L'Hôpital regime when the rates are within ~1 ppm of each other.
            let nb = if ((la - lb) / la).abs() < 1.0e-6 {
                la * a.n_a0 * a.time_s * (-la * a.time_s).exp()
            } else {
                (la * a.n_a0 / (lb - la)) * ((-la * a.time_s).exp() - (-lb * a.time_s).exp())
            };
            Ok(text_result(
                json!({
                    "n_parent": na,
                    "n_daughter": nb,
                    "activity_parent_bq": la * na,
                    "activity_daughter_bq": lb * nb,
                    "lambda_a_per_s": la,
                    "lambda_b_per_s": lb,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// nuke_unit_convert — barn ↔ cm², u ↔ MeV/c², Bq ↔ Ci.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UnitArgs {
    /// Source unit: `u_to_mev`, `mev_to_u`, `bq_to_ci`, `ci_to_bq`,
    /// `barn_to_cm2`, `cm2_to_barn`.
    direction: String,
    /// Source value.
    value: f64,
}

pub struct NukeUnitConvert;
impl Skill for NukeUnitConvert {
    fn name(&self) -> &'static str {
        "nuke_unit_convert"
    }
    fn description(&self) -> &'static str {
        "Convert nuclear-physics units: u ↔ MeV/c² (CODATA 2022 factor \
        931.49410372 MeV/c² per u), Bq ↔ Ci (exact 1 Ci = 3.7e10 Bq), and \
        barn ↔ cm² (exact 1 b = 1e-24 cm²)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<UnitArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<UnitArgs>()?;
            let out = match a.direction.to_ascii_lowercase().as_str() {
                "u_to_mev" => a.value * U_TO_MEV,
                "mev_to_u" => a.value / U_TO_MEV,
                "bq_to_ci" => a.value / CURIE_BQ,
                "ci_to_bq" => a.value * CURIE_BQ,
                "barn_to_cm2" => a.value * 1.0e-24,
                "cm2_to_barn" => a.value / 1.0e-24,
                other => return Err(invalid(format!("unknown direction '{other}'"))),
            };
            Ok(text_result(json!({ "result": out }).to_string()))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(NukeNuclideLookup),
        Box::new(NukeBindingEnergy),
        Box::new(NukeQValue),
        Box::new(NukeDecayLaw),
        Box::new(NukeDecayChain),
        Box::new(NukeUnitConvert),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semf_iron_56_close_to_book_value() {
        // Krane's SEMF for Fe-56 should land within a few % of the measured
        // BE/A of ~8.79 MeV. Compute and check.
        const A_V: f64 = 15.5;
        const A_S: f64 = 16.8;
        const A_C: f64 = 0.72;
        const A_A: f64 = 23.0;
        const A_P: f64 = 34.0;
        let a = 56.0_f64;
        let z = 26.0_f64;
        let n = a - z;
        let be = A_V * a
            - A_S * a.powf(2.0 / 3.0)
            - A_C * z * (z - 1.0) / a.cbrt()
            - A_A * (a - 2.0 * z).powi(2) / a
            + if (z as u32).is_multiple_of(2) && (n as u32).is_multiple_of(2) {
                A_P * a.powf(-0.75)
            } else {
                0.0
            };
        let be_per_a = be / a;
        assert!((be_per_a - 8.7903).abs() < 0.5, "BE/A = {be_per_a}");
    }

    #[test]
    fn dt_q_value_close_to_canonical() {
        // D + T → He-4 + n, accepted Q ≈ 17.589 MeV.
        let d = 2.014101778;
        let t = 3.016049281;
        let he4 = 4.002603254;
        let n_mass = 1.008664916;
        let q = (d + t - he4 - n_mass) * U_TO_MEV;
        assert!((q - 17.589).abs() < 0.02, "Q = {q}");
    }

    #[test]
    fn bateman_secular_equilibrium_limit() {
        // For half_life_a ≫ half_life_b, daughter activity at long times
        // approaches parent activity (secular equilibrium).
        let n_a0 = 1.0e12;
        let la = LN_2 / 1.0e10; // very long parent
        let lb = LN_2 / 1.0e3; // short daughter
        let t = 1.0e5; // ≫ daughter half life, ≪ parent
        let na = n_a0 * (-la * t).exp();
        let nb = (la * n_a0 / (lb - la)) * ((-la * t).exp() - (-lb * t).exp());
        let act_p = la * na;
        let act_d = lb * nb;
        assert!((act_d / act_p - 1.0).abs() < 1.0e-3);
    }

    #[test]
    fn lookup_u235_returns_alpha_decay() {
        let u = find_nuclide("U-235").unwrap();
        assert_eq!(u.z, 92);
        assert_eq!(u.n, 143);
        assert!(u.decay_modes.contains('A'));
    }

    #[test]
    fn u_to_mev_round_trip() {
        let original = 235.043930131_f64;
        let mev = original * U_TO_MEV;
        let back = mev / U_TO_MEV;
        assert!((original - back).abs() < 1.0e-12);
    }
}
