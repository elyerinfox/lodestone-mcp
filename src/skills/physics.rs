//! Physics skill (local, no network): `physics_formula` / `physics_formula_list`
//! compute named physics formulas across mechanics, gravitation, electromagnetism,
//! thermodynamics, waves/optics, relativity, atomic/nuclear, and fluids;
//! `physical_constant` looks up SI constants; `wave_frequency` converts between a
//! wave's frequency, wavelength, and period. Angles in degrees; SI units.
#![allow(clippy::inconsistent_digit_grouping)]

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::arithmetic::fmt_num;
use crate::skills::formula::{self, opt, v, Args, Formula};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

use std::f64::consts::{LN_2, PI};

// ---- Physical constants (SI, CODATA-ish) ----------------------------------
const C: f64 = 299_792_458.0; // speed of light, m/s
const G: f64 = 6.674_30e-11; // gravitational constant, m³/(kg·s²)
const H: f64 = 6.626_070_15e-34; // Planck, J·s
const K_E: f64 = 8.987_551_792_3e9; // Coulomb, N·m²/C²
const E_CHARGE: f64 = 1.602_176_634e-19; // elementary charge, C
const EPSILON0: f64 = 8.854_187_812_8e-12; // vacuum permittivity, F/m
const MU0: f64 = 1.256_637_062_12e-6; // vacuum permeability, N/A²
const K_B: f64 = 1.380_649e-23; // Boltzmann, J/K
const R_GAS: f64 = 8.314_462_618; // molar gas constant, J/(mol·K)
const N_A: f64 = 6.022_140_76e23; // Avogadro, 1/mol
const SIGMA_SB: f64 = 5.670_374_419e-8; // Stefan-Boltzmann, W/(m²·K⁴)
const G_ACCEL: f64 = 9.806_65; // standard gravity, m/s²
const R_RYDBERG: f64 = 1.097_373_156_816e7; // Rydberg, 1/m
const M_E: f64 = 9.109_383_7015e-31; // electron mass, kg
const M_P: f64 = 1.672_621_923_69e-27; // proton mass, kg
const ATM: f64 = 101_325.0; // standard atmosphere, Pa

#[rustfmt::skip]
static FORMULAS: LazyLock<Vec<Formula>> = LazyLock::new(|| {
    vec![
        // ---- Mechanics ----
        Formula { id: "kinetic_energy", category: "mechanics", summary: "Kinetic energy: KE = ½·m·v²", inputs: vec![v("m","kg"), v("v","m/s")], out: v("KE","J"), eval: |a| 0.5*a["m"]*a["v"].powi(2) },
        Formula { id: "momentum", category: "mechanics", summary: "Momentum: p = m·v", inputs: vec![v("m","kg"), v("v","m/s")], out: v("p","kg·m/s"), eval: |a| a["m"]*a["v"] },
        Formula { id: "newton_second_law", category: "mechanics", summary: "Newton's 2nd law: F = m·a", inputs: vec![v("m","kg"), v("a","m/s²")], out: v("F","N"), eval: |a| a["m"]*a["a"] },
        Formula { id: "weight", category: "mechanics", summary: "Weight: W = m·g (g≈9.80665, override with g)", inputs: vec![v("m","kg")], out: v("W","N"), eval: |a| a["m"]*opt(a,"g",G_ACCEL) },
        Formula { id: "gravitational_pe", category: "mechanics", summary: "Gravitational PE near Earth: PE = m·g·h (g optional)", inputs: vec![v("m","kg"), v("h","m")], out: v("PE","J"), eval: |a| a["m"]*opt(a,"g",G_ACCEL)*a["h"] },
        Formula { id: "work", category: "mechanics", summary: "Work: W = F·d·cos(theta°) (theta optional, default 0)", inputs: vec![v("F","N"), v("d","m")], out: v("W","J"), eval: |a| a["F"]*a["d"]*opt(a,"theta",0.0).to_radians().cos() },
        Formula { id: "power_time", category: "mechanics", summary: "Power: P = W/t", inputs: vec![v("W","J"), v("t","s")], out: v("P","W"), eval: |a| a["W"]/a["t"] },
        Formula { id: "power_force_velocity", category: "mechanics", summary: "Power: P = F·v", inputs: vec![v("F","N"), v("v","m/s")], out: v("P","W"), eval: |a| a["F"]*a["v"] },
        Formula { id: "impulse", category: "mechanics", summary: "Impulse: J = F·Δt", inputs: vec![v("F","N"), v("t","s")], out: v("J","N·s"), eval: |a| a["F"]*a["t"] },
        Formula { id: "pressure", category: "mechanics", summary: "Pressure: P = F/A", inputs: vec![v("F","N"), v("A","m²")], out: v("P","Pa"), eval: |a| a["F"]/a["A"] },
        Formula { id: "density", category: "mechanics", summary: "Density: ρ = m/V", inputs: vec![v("m","kg"), v("V","m³")], out: v("rho","kg/m³"), eval: |a| a["m"]/a["V"] },
        Formula { id: "kinematics_velocity", category: "mechanics", summary: "v = u + a·t", inputs: vec![v("u","m/s"), v("a","m/s²"), v("t","s")], out: v("v","m/s"), eval: |a| a["u"]+a["a"]*a["t"] },
        Formula { id: "kinematics_displacement", category: "mechanics", summary: "s = u·t + ½·a·t²", inputs: vec![v("u","m/s"), v("a","m/s²"), v("t","s")], out: v("s","m"), eval: |a| a["u"]*a["t"]+0.5*a["a"]*a["t"].powi(2) },
        Formula { id: "kinematics_velocity_sq", category: "mechanics", summary: "v = √(u² + 2·a·s)", inputs: vec![v("u","m/s"), v("a","m/s²"), v("s","m")], out: v("v","m/s"), eval: |a| (a["u"].powi(2)+2.0*a["a"]*a["s"]).sqrt() },
        Formula { id: "centripetal_acceleration", category: "mechanics", summary: "Centripetal accel: a = v²/r", inputs: vec![v("v","m/s"), v("r","m")], out: v("a","m/s²"), eval: |a| a["v"].powi(2)/a["r"] },
        Formula { id: "centripetal_force", category: "mechanics", summary: "Centripetal force: F = m·v²/r", inputs: vec![v("m","kg"), v("v","m/s"), v("r","m")], out: v("F","N"), eval: |a| a["m"]*a["v"].powi(2)/a["r"] },
        Formula { id: "hookes_law", category: "mechanics", summary: "Hooke's law (magnitude): F = k·x", inputs: vec![v("k","N/m"), v("x","m")], out: v("F","N"), eval: |a| a["k"]*a["x"] },
        Formula { id: "spring_pe", category: "mechanics", summary: "Spring PE: PE = ½·k·x²", inputs: vec![v("k","N/m"), v("x","m")], out: v("PE","J"), eval: |a| 0.5*a["k"]*a["x"].powi(2) },
        Formula { id: "spring_period", category: "mechanics", summary: "Mass-spring period: T = 2π·√(m/k)", inputs: vec![v("m","kg"), v("k","N/m")], out: v("T","s"), eval: |a| 2.0*PI*(a["m"]/a["k"]).sqrt() },
        Formula { id: "pendulum_period", category: "mechanics", summary: "Simple pendulum period: T = 2π·√(L/g) (g optional)", inputs: vec![v("L","m")], out: v("T","s"), eval: |a| 2.0*PI*(a["L"]/opt(a,"g",G_ACCEL)).sqrt() },
        Formula { id: "torque", category: "mechanics", summary: "Torque: τ = r·F·sin(theta°) (theta optional, default 90)", inputs: vec![v("r","m"), v("F","N")], out: v("tau","N·m"), eval: |a| a["r"]*a["F"]*opt(a,"theta",90.0).to_radians().sin() },
        Formula { id: "angular_velocity", category: "mechanics", summary: "Angular velocity: ω = 2π·f", inputs: vec![v("f","Hz")], out: v("omega","rad/s"), eval: |a| 2.0*PI*a["f"] },
        Formula { id: "rotational_ke", category: "mechanics", summary: "Rotational KE: KE = ½·I·ω²", inputs: vec![v("I","kg·m²"), v("omega","rad/s")], out: v("KE","J"), eval: |a| 0.5*a["I"]*a["omega"].powi(2) },
        Formula { id: "moment_of_inertia_point", category: "mechanics", summary: "Point-mass moment of inertia: I = m·r²", inputs: vec![v("m","kg"), v("r","m")], out: v("I","kg·m²"), eval: |a| a["m"]*a["r"].powi(2) },

        // ---- Gravitation ----
        Formula { id: "newton_gravitation", category: "gravitation", summary: "Newton's gravitation: F = G·m₁·m₂/r²", inputs: vec![v("m1","kg"), v("m2","kg"), v("r","m")], out: v("F","N"), eval: |a| G*a["m1"]*a["m2"]/a["r"].powi(2) },
        Formula { id: "gravitational_field", category: "gravitation", summary: "Gravitational field: g = G·M/r²", inputs: vec![v("M","kg"), v("r","m")], out: v("g","m/s²"), eval: |a| G*a["M"]/a["r"].powi(2) },
        Formula { id: "gravitational_potential_energy", category: "gravitation", summary: "Gravitational PE: U = -G·m₁·m₂/r", inputs: vec![v("m1","kg"), v("m2","kg"), v("r","m")], out: v("U","J"), eval: |a| -G*a["m1"]*a["m2"]/a["r"] },
        Formula { id: "escape_velocity", category: "gravitation", summary: "Escape velocity: v = √(2·G·M/r)", inputs: vec![v("M","kg"), v("r","m")], out: v("v","m/s"), eval: |a| (2.0*G*a["M"]/a["r"]).sqrt() },
        Formula { id: "orbital_velocity", category: "gravitation", summary: "Circular orbital velocity: v = √(G·M/r)", inputs: vec![v("M","kg"), v("r","m")], out: v("v","m/s"), eval: |a| (G*a["M"]/a["r"]).sqrt() },
        Formula { id: "kepler_third_period", category: "gravitation", summary: "Kepler's 3rd: T = 2π·√(a³/(G·M))", inputs: vec![v("a","m"), v("M","kg")], out: v("T","s"), eval: |a| 2.0*PI*(a["a"].powi(3)/(G*a["M"])).sqrt() },
        Formula { id: "schwarzschild_radius", category: "gravitation", summary: "Schwarzschild radius: r = 2·G·M/c²", inputs: vec![v("M","kg")], out: v("r","m"), eval: |a| 2.0*G*a["M"]/C.powi(2) },

        // ---- Electromagnetism ----
        Formula { id: "ohms_law_voltage", category: "electromagnetism", summary: "Ohm's law: V = I·R", inputs: vec![v("I","A"), v("R","Ω")], out: v("V","V"), eval: |a| a["I"]*a["R"] },
        Formula { id: "ohms_law_current", category: "electromagnetism", summary: "Ohm's law: I = V/R", inputs: vec![v("V","V"), v("R","Ω")], out: v("I","A"), eval: |a| a["V"]/a["R"] },
        Formula { id: "ohms_law_resistance", category: "electromagnetism", summary: "Ohm's law: R = V/I", inputs: vec![v("V","V"), v("I","A")], out: v("R","Ω"), eval: |a| a["V"]/a["I"] },
        Formula { id: "power_electrical", category: "electromagnetism", summary: "Electrical power: P = V·I", inputs: vec![v("V","V"), v("I","A")], out: v("P","W"), eval: |a| a["V"]*a["I"] },
        Formula { id: "power_resistive", category: "electromagnetism", summary: "Resistive power: P = I²·R", inputs: vec![v("I","A"), v("R","Ω")], out: v("P","W"), eval: |a| a["I"].powi(2)*a["R"] },
        Formula { id: "power_voltage_resistance", category: "electromagnetism", summary: "Power: P = V²/R", inputs: vec![v("V","V"), v("R","Ω")], out: v("P","W"), eval: |a| a["V"].powi(2)/a["R"] },
        Formula { id: "coulombs_law", category: "electromagnetism", summary: "Coulomb's law: F = k·q₁·q₂/r²", inputs: vec![v("q1","C"), v("q2","C"), v("r","m")], out: v("F","N"), eval: |a| K_E*a["q1"]*a["q2"]/a["r"].powi(2) },
        Formula { id: "electric_field_point", category: "electromagnetism", summary: "Field of a point charge: E = k·Q/r²", inputs: vec![v("Q","C"), v("r","m")], out: v("E","N/C"), eval: |a| K_E*a["Q"]/a["r"].powi(2) },
        Formula { id: "electric_field_force", category: "electromagnetism", summary: "Field from force: E = F/q", inputs: vec![v("F","N"), v("q","C")], out: v("E","N/C"), eval: |a| a["F"]/a["q"] },
        Formula { id: "electric_potential_point", category: "electromagnetism", summary: "Potential of a point charge: V = k·Q/r", inputs: vec![v("Q","C"), v("r","m")], out: v("V","V"), eval: |a| K_E*a["Q"]/a["r"] },
        Formula { id: "capacitance", category: "electromagnetism", summary: "Capacitance: C = Q/V", inputs: vec![v("Q","C"), v("V","V")], out: v("C","F"), eval: |a| a["Q"]/a["V"] },
        Formula { id: "capacitor_energy", category: "electromagnetism", summary: "Capacitor energy: E = ½·C·V²", inputs: vec![v("C","F"), v("V","V")], out: v("E","J"), eval: |a| 0.5*a["C"]*a["V"].powi(2) },
        Formula { id: "resistors_series", category: "electromagnetism", summary: "Series resistors: R = R₁ + R₂", inputs: vec![v("r1","Ω"), v("r2","Ω")], out: v("R","Ω"), eval: |a| a["r1"]+a["r2"] },
        Formula { id: "resistors_parallel", category: "electromagnetism", summary: "Parallel resistors: R = R₁·R₂/(R₁+R₂)", inputs: vec![v("r1","Ω"), v("r2","Ω")], out: v("R","Ω"), eval: |a| a["r1"]*a["r2"]/(a["r1"]+a["r2"]) },

        // ---- Waves / optics ----
        Formula { id: "wave_speed", category: "waves", summary: "Wave speed: v = f·λ", inputs: vec![v("f","Hz"), v("lambda","m")], out: v("v","m/s"), eval: |a| a["f"]*a["lambda"] },
        Formula { id: "frequency_from_period", category: "waves", summary: "Frequency: f = 1/T", inputs: vec![v("T","s")], out: v("f","Hz"), eval: |a| 1.0/a["T"] },
        Formula { id: "photon_energy_frequency", category: "waves", summary: "Photon energy: E = h·f", inputs: vec![v("f","Hz")], out: v("E","J"), eval: |a| H*a["f"] },
        Formula { id: "photon_energy_wavelength", category: "waves", summary: "Photon energy: E = h·c/λ", inputs: vec![v("lambda","m")], out: v("E","J"), eval: |a| H*C/a["lambda"] },
        Formula { id: "de_broglie", category: "waves", summary: "de Broglie wavelength: λ = h/p", inputs: vec![v("p","kg·m/s")], out: v("lambda","m"), eval: |a| H/a["p"] },
        Formula { id: "friis_path_loss", category: "waves", summary: "Free-space (Friis) path loss: FSPL = 20·log10(4·π·d·f/c)", inputs: vec![v("f","Hz"), v("d","m")], out: v("FSPL","dB"), eval: |a| 20.0 * (4.0*PI*a["d"]*a["f"]/C).log10() },
        Formula { id: "shannon_hartley", category: "waves", summary: "Shannon-Hartley channel capacity: C = B·log2(1 + 10^(SNR_dB/10))", inputs: vec![v("B","Hz"), v("SNR_dB","dB")], out: v("C","bit/s"), eval: |a| a["B"] * (1.0 + 10f64.powf(a["SNR_dB"]/10.0)).log2() },
        Formula { id: "snells_law_angle", category: "optics", summary: "Snell's law: θ₂ = asin(n₁·sin(theta1°)/n₂)", inputs: vec![v("n1",""), v("n2",""), v("theta1","deg")], out: v("theta2","deg"), eval: |a| (a["n1"]*a["theta1"].to_radians().sin()/a["n2"]).asin().to_degrees() },
        Formula { id: "thin_lens_image_distance", category: "optics", summary: "Thin lens: 1/f = 1/do + 1/di → di", inputs: vec![v("f","m"), v("do","m")], out: v("di","m"), eval: |a| 1.0/(1.0/a["f"] - 1.0/a["do"]) },
        Formula { id: "magnification", category: "optics", summary: "Magnification: m = -di/do", inputs: vec![v("di","m"), v("do","m")], out: v("m",""), eval: |a| -a["di"]/a["do"] },

        // ---- Thermodynamics ----
        Formula { id: "ideal_gas_pressure", category: "thermodynamics", summary: "Ideal gas: P = n·R·T/V", inputs: vec![v("n","mol"), v("T","K"), v("V","m³")], out: v("P","Pa"), eval: |a| a["n"]*R_GAS*a["T"]/a["V"] },
        Formula { id: "ideal_gas_volume", category: "thermodynamics", summary: "Ideal gas: V = n·R·T/P", inputs: vec![v("n","mol"), v("T","K"), v("P","Pa")], out: v("V","m³"), eval: |a| a["n"]*R_GAS*a["T"]/a["P"] },
        Formula { id: "heat_energy", category: "thermodynamics", summary: "Sensible heat: Q = m·c·ΔT", inputs: vec![v("m","kg"), v("c","J/(kg·K)"), v("dT","K")], out: v("Q","J"), eval: |a| a["m"]*a["c"]*a["dT"] },
        Formula { id: "carnot_efficiency", category: "thermodynamics", summary: "Carnot efficiency: η = 1 - Tc/Th", inputs: vec![v("Tc","K"), v("Th","K")], out: v("eta",""), eval: |a| 1.0 - a["Tc"]/a["Th"] },
        Formula { id: "thermal_expansion_linear", category: "thermodynamics", summary: "Linear expansion: ΔL = α·L₀·ΔT", inputs: vec![v("alpha","1/K"), v("L0","m"), v("dT","K")], out: v("dL","m"), eval: |a| a["alpha"]*a["L0"]*a["dT"] },
        Formula { id: "stefan_boltzmann", category: "thermodynamics", summary: "Radiated power: P = ε·σ·A·T⁴ (eps optional, default 1)", inputs: vec![v("A","m²"), v("T","K")], out: v("P","W"), eval: |a| opt(a,"eps",1.0)*SIGMA_SB*a["A"]*a["T"].powi(4) },
        Formula { id: "thermal_noise_kTB", category: "thermodynamics", summary: "Thermal noise power: N_dBm = 10·log10(k·T·B·1000) + NF (NF optional dB, default 0)", inputs: vec![v("T","K"), v("B","Hz")], out: v("N","dBm"), eval: |a| 10.0*(K_B*a["T"]*a["B"]*1000.0).log10() + opt(a,"NF",0.0) },

        // ---- Relativity ----
        Formula { id: "mass_energy", category: "relativity", summary: "Mass-energy: E = m·c²", inputs: vec![v("m","kg")], out: v("E","J"), eval: |a| a["m"]*C.powi(2) },
        Formula { id: "lorentz_factor", category: "relativity", summary: "Lorentz factor: γ = 1/√(1-v²/c²)", inputs: vec![v("v","m/s")], out: v("gamma",""), eval: |a| 1.0/(1.0 - (a["v"]/C).powi(2)).sqrt() },
        Formula { id: "time_dilation", category: "relativity", summary: "Time dilation: Δt = γ·Δt₀", inputs: vec![v("t0","s"), v("v","m/s")], out: v("t","s"), eval: |a| a["t0"]/(1.0 - (a["v"]/C).powi(2)).sqrt() },
        Formula { id: "length_contraction", category: "relativity", summary: "Length contraction: L = L₀·√(1-v²/c²)", inputs: vec![v("L0","m"), v("v","m/s")], out: v("L","m"), eval: |a| a["L0"]*(1.0 - (a["v"]/C).powi(2)).sqrt() },
        Formula { id: "relativistic_momentum", category: "relativity", summary: "Relativistic momentum: p = γ·m·v", inputs: vec![v("m","kg"), v("v","m/s")], out: v("p","kg·m/s"), eval: |a| a["m"]*a["v"]/(1.0 - (a["v"]/C).powi(2)).sqrt() },

        // ---- Atomic / nuclear ----
        Formula { id: "rydberg_wavelength", category: "atomic", summary: "Rydberg: 1/λ = R(1/n₁² - 1/n₂²) → λ (n₂>n₁)", inputs: vec![v("n1",""), v("n2","")], out: v("lambda","m"), eval: |a| 1.0/(R_RYDBERG*(1.0/a["n1"].powi(2) - 1.0/a["n2"].powi(2))) },
        Formula { id: "radioactive_decay", category: "nuclear", summary: "Decay: N = N₀·e^(-λ·t)", inputs: vec![v("n0",""), v("lambda","1/s"), v("t","s")], out: v("N",""), eval: |a| a["n0"]*(-a["lambda"]*a["t"]).exp() },
        Formula { id: "half_life_remaining", category: "nuclear", summary: "Remaining after t: N = N₀·(½)^(t/t_half)", inputs: vec![v("n0",""), v("t","s"), v("t_half","s")], out: v("N",""), eval: |a| a["n0"]*0.5_f64.powf(a["t"]/a["t_half"]) },
        Formula { id: "decay_constant_from_half_life", category: "nuclear", summary: "Decay constant: λ = ln2 / t_half", inputs: vec![v("t_half","s")], out: v("lambda","1/s"), eval: |a| LN_2/a["t_half"] },

        // ---- Fluids ----
        Formula { id: "continuity_velocity", category: "fluids", summary: "Continuity: v₂ = A₁·v₁/A₂", inputs: vec![v("a1","m²"), v("v1","m/s"), v("a2","m²")], out: v("v2","m/s"), eval: |a| a["a1"]*a["v1"]/a["a2"] },
        Formula { id: "buoyant_force", category: "fluids", summary: "Buoyancy: F = ρ·V·g (g optional)", inputs: vec![v("rho","kg/m³"), v("V","m³")], out: v("F","N"), eval: |a| a["rho"]*a["V"]*opt(a,"g",G_ACCEL) },
        Formula { id: "volumetric_flow_rate", category: "fluids", summary: "Flow rate: Q = A·v", inputs: vec![v("A","m²"), v("v","m/s")], out: v("Q","m³/s"), eval: |a| a["A"]*a["v"] },
        Formula { id: "hydrostatic_pressure", category: "fluids", summary: "Hydrostatic pressure: P = ρ·g·h (g optional)", inputs: vec![v("rho","kg/m³"), v("h","m")], out: v("P","Pa"), eval: |a| a["rho"]*opt(a,"g",G_ACCEL)*a["h"] },
        Formula { id: "dynamic_pressure", category: "fluids", summary: "Dynamic pressure: q = ½·ρ·v²", inputs: vec![v("rho","kg/m³"), v("v","m/s")], out: v("q","Pa"), eval: |a| 0.5*a["rho"]*a["v"].powi(2) },
    ]
});

/// Physical constants table (symbol, name, value, unit).
static CONSTANTS: &[(&str, &str, f64, &str)] = &[
    ("c", "speed of light in vacuum", C, "m/s"),
    ("G", "gravitational constant", G, "m³/(kg·s²)"),
    ("h", "Planck constant", H, "J·s"),
    ("hbar", "reduced Planck constant", H / (2.0 * PI), "J·s"),
    ("k_e", "Coulomb constant", K_E, "N·m²/C²"),
    ("e", "elementary charge", E_CHARGE, "C"),
    ("epsilon_0", "vacuum permittivity", EPSILON0, "F/m"),
    ("mu_0", "vacuum permeability", MU0, "N/A²"),
    ("k_B", "Boltzmann constant", K_B, "J/K"),
    ("R", "molar gas constant", R_GAS, "J/(mol·K)"),
    ("N_A", "Avogadro constant", N_A, "1/mol"),
    ("sigma", "Stefan-Boltzmann constant", SIGMA_SB, "W/(m²·K⁴)"),
    ("g", "standard gravity", G_ACCEL, "m/s²"),
    ("R_inf", "Rydberg constant", R_RYDBERG, "1/m"),
    ("m_e", "electron mass", M_E, "kg"),
    ("m_p", "proton mass", M_P, "kg"),
    ("atm", "standard atmosphere", ATM, "Pa"),
];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormulaArgs {
    /// Formula id (see `physics_formula_list`), e.g. `kinetic_energy`, `mass_energy`.
    name: String,
    /// Variable values, e.g. `{"m": 2, "v": 3}`. SI units; angles in degrees.
    #[serde(default)]
    args: Args,
}

pub struct PhysicsFormula;
impl Skill for PhysicsFormula {
    fn name(&self) -> &'static str {
        "physics_formula"
    }
    fn description(&self) -> &'static str {
        "Compute a named physics formula (local): mechanics, gravitation, electromagnetism, \
        thermodynamics, waves/optics, relativity, atomic/nuclear, fluids. Pass `name` (an id from \
        physics_formula_list, e.g. kinetic_energy, ohms_law_voltage, ideal_gas_pressure, \
        mass_energy) and `args` as a {var: value} map; SI units, angles in degrees."
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
                title: "Kinetic energy",
                args: r#"{"name": "kinetic_energy", "args": {"m": 2, "v": 3}}"#,
                note: Some("Returns ½·m·v² = 9 J."),
            },
            SkillExample {
                title: "Ohm's law (find voltage)",
                args: r#"{"name": "ohms_law_voltage", "args": {"I": 2, "R": 3}}"#,
                note: Some("V = I·R = 6 V."),
            },
            SkillExample {
                title: "Mass-energy equivalence",
                args: r#"{"name": "mass_energy", "args": {"m": 1}}"#,
                note: Some("E = m·c² for 1 kg ≈ 9e16 J."),
            },
            SkillExample {
                title: "Ideal gas with optional inputs",
                args: r#"{"name": "ideal_gas_pressure", "args": {"n": 1, "T": 273.15, "V": 0.022414}}"#,
                note: Some("SI units throughout; angles where applicable are in degrees."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute a named physics formula by id with explicit SI inputs.",
            "Avoid hand-typing the formula when its id is in the catalog.",
            "Plug values into a closed-form mechanics / EM / thermo equation.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListArgs {
    /// Optional filter: a category (mechanics, electromagnetism, …) or an id/equation substring.
    #[serde(default)]
    filter: Option<String>,
}

pub struct PhysicsFormulaList;
impl Skill for PhysicsFormulaList {
    fn name(&self) -> &'static str {
        "physics_formula_list"
    }
    fn description(&self) -> &'static str {
        "List the named physics formulas (id, equation, signature), grouped by category. Optional \
        `filter` matches a category or id/equation substring. Feed an id to physics_formula."
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
                title: "Every formula",
                args: r#"{}"#,
                note: Some("Lists every formula grouped by category."),
            },
            SkillExample {
                title: "Filter by category",
                args: r#"{"filter": "relativity"}"#,
                note: Some("Matches category name or id / equation substring."),
            },
            SkillExample {
                title: "Find Ohm's-law variants",
                args: r#"{"filter": "ohms_law"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Discover which formula id to feed `physics_formula`.",
            "Browse formulas in a category before picking one.",
            "Search by keyword for a specific equation.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConstArgs {
    /// Optional symbol or name substring (e.g. `c`, `planck`, `boltzmann`). Omit for all.
    #[serde(default)]
    name: Option<String>,
}

pub struct PhysicalConstant;
impl Skill for PhysicalConstant {
    fn name(&self) -> &'static str {
        "physical_constant"
    }
    fn description(&self) -> &'static str {
        "Look up SI physical constants (speed of light, G, Planck, Boltzmann, gas constant, \
        elementary charge, Avogadro, Stefan-Boltzmann, …). Optional `name` filters by symbol or \
        name substring; omit to list all."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConstArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ConstArgs>()?;
            let q = args.name.as_deref().map(|s| s.trim().to_ascii_lowercase());
            let rows: Vec<&(&str, &str, f64, &str)> = CONSTANTS
                .iter()
                .filter(|(sym, name, _, _)| match &q {
                    None => true,
                    Some(q) => {
                        sym.to_ascii_lowercase().contains(q.as_str())
                            || name.to_ascii_lowercase().contains(q.as_str())
                    }
                })
                .collect();
            if rows.is_empty() {
                return Err(invalid(format!(
                    "no physical constant matches '{}'",
                    q.unwrap_or_default()
                )));
            }
            let body: Vec<String> = rows
                .iter()
                .map(|(sym, name, val, unit)| {
                    format!("  {sym} = {} {unit}  ({name})", formula::fmt_num(*val))
                })
                .collect();
            Ok(text_result(format!(
                "Physical constants:\n{}",
                body.join("\n")
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "List all constants",
                args: r#"{}"#,
                note: Some("Returns every SI constant with symbol, value, and unit."),
            },
            SkillExample {
                title: "Filter by name substring",
                args: r#"{"name": "speed of light"}"#,
                note: Some(
                    "`name` is a case-insensitive substring match against both symbol and name, \
                     so narrow filters (`speed of light`, `boltzmann`) return one entry; broad \
                     filters (`c`, `e`) return every entry that contains that letter.",
                ),
            },
            SkillExample {
                title: "Look up by name substring",
                args: r#"{"name": "planck"}"#,
                note: Some("Matches both `h` and `hbar`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Look up the numeric value of a fundamental constant.",
            "Confirm a unit / symbol before using a constant in a formula.",
            "Browse the available SI constants.",
        ]
    }
}

// ---- Wave frequency ↔ wavelength ↔ period (v = f·λ) ----

const SPEED_OF_LIGHT: f64 = C;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaveArgs {
    /// Frequency in hertz. Give exactly one of `frequency_hz` or `wavelength_m`.
    #[serde(default)]
    frequency_hz: Option<f64>,
    /// Wavelength in metres. Give exactly one of `frequency_hz` or `wavelength_m`.
    #[serde(default)]
    wavelength_m: Option<f64>,
    /// Wave speed in m/s. Omit for the speed of light (use ~343 for sound in air).
    #[serde(default)]
    speed_m_s: Option<f64>,
}

/// SI-scale a value with a unit (e.g. 1.2e6 Hz → "1.2 MHz").
fn si(value: f64, unit: &str) -> String {
    let abs = value.abs();
    let (scaled, prefix) = if abs >= 1e9 {
        (value / 1e9, "G")
    } else if abs >= 1e6 {
        (value / 1e6, "M")
    } else if abs >= 1e3 {
        (value / 1e3, "k")
    } else if abs >= 1.0 || abs == 0.0 {
        (value, "")
    } else if abs >= 1e-3 {
        (value * 1e3, "m")
    } else if abs >= 1e-6 {
        (value * 1e6, "µ")
    } else {
        (value * 1e9, "n")
    };
    format!("{} {prefix}{unit}", fmt_num((scaled * 1e6).round() / 1e6))
}

pub struct WaveFrequency;
impl Skill for WaveFrequency {
    fn name(&self) -> &'static str {
        "wave_frequency"
    }
    fn description(&self) -> &'static str {
        "Convert between a wave's frequency, wavelength, and period using v = f·λ (local, no \
        network). Give exactly one of frequency_hz or wavelength_m; speed_m_s defaults to the speed \
        of light (set ~343 for sound in air). Returns frequency, wavelength, and period."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WaveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, a) = ctx.parse::<WaveArgs>()?;
            let v = a.speed_m_s.unwrap_or(SPEED_OF_LIGHT);
            if v <= 0.0 {
                return Err(invalid("speed_m_s must be positive"));
            }
            let (freq, wavelength) = match (a.frequency_hz, a.wavelength_m) {
                (Some(f), None) if f > 0.0 => (f, v / f),
                (None, Some(w)) if w > 0.0 => (v / w, w),
                (Some(_), Some(_)) => {
                    return Err(invalid("give only one of frequency_hz / wavelength_m"))
                }
                _ => return Err(invalid("give a positive frequency_hz or wavelength_m")),
            };
            let period = 1.0 / freq;
            Ok(text_result(format!(
                "wave (speed {} m/s):\n  frequency:  {}\n  wavelength: {}\n  period:     {}",
                fmt_num(v),
                si(freq, "Hz"),
                si(wavelength, "m"),
                si(period, "s"),
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "FM radio frequency to wavelength",
                args: r#"{"frequency_hz": 1.0e8}"#,
                note: Some("Defaults to speed of light; 100 MHz → ~3 m wavelength."),
            },
            SkillExample {
                title: "Visible-light wavelength to frequency",
                args: r#"{"wavelength_m": 5.5e-7}"#,
                note: Some("550 nm green light → ~545 THz."),
            },
            SkillExample {
                title: "Sound wave (speed override)",
                args: r#"{"frequency_hz": 440, "speed_m_s": 343}"#,
                note: Some("Concert-A through air; wavelength ≈ 0.78 m."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert between a wave's frequency, wavelength, and period.",
            "Compute the wavelength of a radio / optical signal at the speed of light.",
            "Work out acoustic wavelengths by overriding the propagation speed.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::ExactlyOne {
            fields: &["frequency_hz", "wavelength_m"],
        }]
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(PhysicsFormula),
        Box::new(PhysicsFormulaList),
        Box::new(PhysicalConstant),
        Box::new(WaveFrequency),
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
    fn all_ids_unique() {
        let mut ids: Vec<&str> = FORMULAS.iter().map(|f| f.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate physics formula id");
    }

    #[test]
    fn mechanics_em_values() {
        assert_eq!(run("kinetic_energy", &[("m", 2.0), ("v", 3.0)]), 9.0);
        assert_eq!(run("ohms_law_voltage", &[("I", 2.0), ("R", 3.0)]), 6.0);
        assert_eq!(run("power_electrical", &[("V", 120.0), ("I", 0.5)]), 60.0);
    }

    #[test]
    fn relativity_thermo_optional() {
        let e = run("mass_energy", &[("m", 1.0)]);
        assert!((e - 8.987_551_787e16).abs() / e < 1e-6);
        let p = run(
            "ideal_gas_pressure",
            &[("n", 1.0), ("T", 273.15), ("V", 0.022_414)],
        );
        assert!((p - 101_325.0).abs() < 2000.0, "p={p}");
        // weight uses g default.
        assert!((run("weight", &[("m", 1.0)]) - G_ACCEL).abs() < 1e-9);
    }
}
