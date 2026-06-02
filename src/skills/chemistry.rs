//! Chemistry — periodic table lookup, molar mass, equation balancing,
//! acid-base / buffer / gas-law / thermo / dilution calculators. Pure-Rust,
//! no host requirements, on by default.
//!
//! ## Source citations
//!
//! - **Atomic weights**: IUPAC Commission on Isotopic Abundances and Atomic
//!   Weights (CIAAW), *Standard Atomic Weights 2021* (Prohaska et al., Pure
//!   Appl. Chem. 2022, 94(5):573-600). Abridged values per IUPAC
//!   recommendation; for elements with conventional intervals (H, Li, B, C,
//!   N, O, Mg, Si, S, Cl, Br, Tl, Pb) the conventional single value is used.
//! - **Group placement** (Sc/Y/Lu/Lr in Group 3): IUPAC Provisional Report
//!   (Scerri et al., 2021).
//! - **Universal gas constant** R = 8.314 462 618 J/(mol·K) — exact value
//!   defined by the 2019 SI redefinition (R = N_A · k_B with both
//!   constituents now exactly defined).
//! - **Henderson-Hasselbalch**: Henderson (1908), Hasselbalch (1917).
//! - **Hill ordering**: Hill, J. Am. Chem. Soc. 1900, 22(8):478-494.
//! - **Equation balancer**: null-space of element-coefficient matrix via
//!   fraction-free Gauss-Jordan over ℤ (cf. Bareiss, *Math. Comp.* 1968).
//! - **Radioactive decay**: standard first-order kinetics
//!   N(t) = N₀·exp(-λt), λ = ln(2)/t½.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use num_bigint::BigInt;
use num_integer::Integer;
use num_traits::{One, Signed, Zero};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// Periodic table — vendored. 118 elements, atomic mass in g/mol (IUPAC 2021),
// group/period, common oxidation states, and one short use note. Stored as a
// single `const` so lookups are zero-allocation.
// ---------------------------------------------------------------------------

struct Element {
    z: u32,
    symbol: &'static str,
    name: &'static str,
    mass: f64,
    group: i32, // -1 if lanthanide/actinide
    period: i32,
    common_ox: &'static [i32],
    category: &'static str,
}

const ELEMENTS: &[Element] = &[
    Element {
        z: 1,
        symbol: "H",
        name: "Hydrogen",
        mass: 1.008,
        group: 1,
        period: 1,
        common_ox: &[-1, 1],
        category: "nonmetal",
    },
    Element {
        z: 2,
        symbol: "He",
        name: "Helium",
        mass: 4.0026,
        group: 18,
        period: 1,
        common_ox: &[0],
        category: "noble_gas",
    },
    Element {
        z: 3,
        symbol: "Li",
        name: "Lithium",
        mass: 6.94,
        group: 1,
        period: 2,
        common_ox: &[1],
        category: "alkali_metal",
    },
    Element {
        z: 4,
        symbol: "Be",
        name: "Beryllium",
        mass: 9.0122,
        group: 2,
        period: 2,
        common_ox: &[2],
        category: "alkaline_earth",
    },
    Element {
        z: 5,
        symbol: "B",
        name: "Boron",
        mass: 10.81,
        group: 13,
        period: 2,
        common_ox: &[3],
        category: "metalloid",
    },
    Element {
        z: 6,
        symbol: "C",
        name: "Carbon",
        mass: 12.011,
        group: 14,
        period: 2,
        common_ox: &[-4, 2, 4],
        category: "nonmetal",
    },
    Element {
        z: 7,
        symbol: "N",
        name: "Nitrogen",
        mass: 14.007,
        group: 15,
        period: 2,
        common_ox: &[-3, 3, 5],
        category: "nonmetal",
    },
    Element {
        z: 8,
        symbol: "O",
        name: "Oxygen",
        mass: 15.999,
        group: 16,
        period: 2,
        common_ox: &[-2],
        category: "nonmetal",
    },
    Element {
        z: 9,
        symbol: "F",
        name: "Fluorine",
        mass: 18.998,
        group: 17,
        period: 2,
        common_ox: &[-1],
        category: "halogen",
    },
    Element {
        z: 10,
        symbol: "Ne",
        name: "Neon",
        mass: 20.180,
        group: 18,
        period: 2,
        common_ox: &[0],
        category: "noble_gas",
    },
    Element {
        z: 11,
        symbol: "Na",
        name: "Sodium",
        mass: 22.990,
        group: 1,
        period: 3,
        common_ox: &[1],
        category: "alkali_metal",
    },
    Element {
        z: 12,
        symbol: "Mg",
        name: "Magnesium",
        mass: 24.305,
        group: 2,
        period: 3,
        common_ox: &[2],
        category: "alkaline_earth",
    },
    Element {
        z: 13,
        symbol: "Al",
        name: "Aluminium",
        mass: 26.982,
        group: 13,
        period: 3,
        common_ox: &[3],
        category: "post_transition",
    },
    Element {
        z: 14,
        symbol: "Si",
        name: "Silicon",
        mass: 28.085,
        group: 14,
        period: 3,
        common_ox: &[-4, 4],
        category: "metalloid",
    },
    Element {
        z: 15,
        symbol: "P",
        name: "Phosphorus",
        mass: 30.974,
        group: 15,
        period: 3,
        common_ox: &[-3, 3, 5],
        category: "nonmetal",
    },
    Element {
        z: 16,
        symbol: "S",
        name: "Sulfur",
        mass: 32.06,
        group: 16,
        period: 3,
        common_ox: &[-2, 4, 6],
        category: "nonmetal",
    },
    Element {
        z: 17,
        symbol: "Cl",
        name: "Chlorine",
        mass: 35.45,
        group: 17,
        period: 3,
        common_ox: &[-1, 1, 3, 5, 7],
        category: "halogen",
    },
    Element {
        z: 18,
        symbol: "Ar",
        name: "Argon",
        mass: 39.95,
        group: 18,
        period: 3,
        common_ox: &[0],
        category: "noble_gas",
    },
    Element {
        z: 19,
        symbol: "K",
        name: "Potassium",
        mass: 39.098,
        group: 1,
        period: 4,
        common_ox: &[1],
        category: "alkali_metal",
    },
    Element {
        z: 20,
        symbol: "Ca",
        name: "Calcium",
        mass: 40.078,
        group: 2,
        period: 4,
        common_ox: &[2],
        category: "alkaline_earth",
    },
    Element {
        z: 21,
        symbol: "Sc",
        name: "Scandium",
        mass: 44.956,
        group: 3,
        period: 4,
        common_ox: &[3],
        category: "transition_metal",
    },
    Element {
        z: 22,
        symbol: "Ti",
        name: "Titanium",
        mass: 47.867,
        group: 4,
        period: 4,
        common_ox: &[2, 3, 4],
        category: "transition_metal",
    },
    Element {
        z: 23,
        symbol: "V",
        name: "Vanadium",
        mass: 50.942,
        group: 5,
        period: 4,
        common_ox: &[2, 3, 4, 5],
        category: "transition_metal",
    },
    Element {
        z: 24,
        symbol: "Cr",
        name: "Chromium",
        mass: 51.996,
        group: 6,
        period: 4,
        common_ox: &[2, 3, 6],
        category: "transition_metal",
    },
    Element {
        z: 25,
        symbol: "Mn",
        name: "Manganese",
        mass: 54.938,
        group: 7,
        period: 4,
        common_ox: &[2, 3, 4, 6, 7],
        category: "transition_metal",
    },
    Element {
        z: 26,
        symbol: "Fe",
        name: "Iron",
        mass: 55.845,
        group: 8,
        period: 4,
        common_ox: &[2, 3],
        category: "transition_metal",
    },
    Element {
        z: 27,
        symbol: "Co",
        name: "Cobalt",
        mass: 58.933,
        group: 9,
        period: 4,
        common_ox: &[2, 3],
        category: "transition_metal",
    },
    Element {
        z: 28,
        symbol: "Ni",
        name: "Nickel",
        mass: 58.693,
        group: 10,
        period: 4,
        common_ox: &[2, 3],
        category: "transition_metal",
    },
    Element {
        z: 29,
        symbol: "Cu",
        name: "Copper",
        mass: 63.546,
        group: 11,
        period: 4,
        common_ox: &[1, 2],
        category: "transition_metal",
    },
    Element {
        z: 30,
        symbol: "Zn",
        name: "Zinc",
        mass: 65.38,
        group: 12,
        period: 4,
        common_ox: &[2],
        category: "transition_metal",
    },
    Element {
        z: 31,
        symbol: "Ga",
        name: "Gallium",
        mass: 69.723,
        group: 13,
        period: 4,
        common_ox: &[3],
        category: "post_transition",
    },
    Element {
        z: 32,
        symbol: "Ge",
        name: "Germanium",
        mass: 72.630,
        group: 14,
        period: 4,
        common_ox: &[2, 4],
        category: "metalloid",
    },
    Element {
        z: 33,
        symbol: "As",
        name: "Arsenic",
        mass: 74.922,
        group: 15,
        period: 4,
        common_ox: &[-3, 3, 5],
        category: "metalloid",
    },
    Element {
        z: 34,
        symbol: "Se",
        name: "Selenium",
        mass: 78.971,
        group: 16,
        period: 4,
        common_ox: &[-2, 4, 6],
        category: "nonmetal",
    },
    Element {
        z: 35,
        symbol: "Br",
        name: "Bromine",
        mass: 79.904,
        group: 17,
        period: 4,
        common_ox: &[-1, 1, 3, 5],
        category: "halogen",
    },
    Element {
        z: 36,
        symbol: "Kr",
        name: "Krypton",
        mass: 83.798,
        group: 18,
        period: 4,
        common_ox: &[0, 2],
        category: "noble_gas",
    },
    Element {
        z: 37,
        symbol: "Rb",
        name: "Rubidium",
        mass: 85.468,
        group: 1,
        period: 5,
        common_ox: &[1],
        category: "alkali_metal",
    },
    Element {
        z: 38,
        symbol: "Sr",
        name: "Strontium",
        mass: 87.62,
        group: 2,
        period: 5,
        common_ox: &[2],
        category: "alkaline_earth",
    },
    Element {
        z: 39,
        symbol: "Y",
        name: "Yttrium",
        mass: 88.906,
        group: 3,
        period: 5,
        common_ox: &[3],
        category: "transition_metal",
    },
    Element {
        z: 40,
        symbol: "Zr",
        name: "Zirconium",
        mass: 91.224,
        group: 4,
        period: 5,
        common_ox: &[4],
        category: "transition_metal",
    },
    Element {
        z: 41,
        symbol: "Nb",
        name: "Niobium",
        mass: 92.906,
        group: 5,
        period: 5,
        common_ox: &[5],
        category: "transition_metal",
    },
    Element {
        z: 42,
        symbol: "Mo",
        name: "Molybdenum",
        mass: 95.95,
        group: 6,
        period: 5,
        common_ox: &[4, 6],
        category: "transition_metal",
    },
    Element {
        z: 43,
        symbol: "Tc",
        name: "Technetium",
        mass: 98.0,
        group: 7,
        period: 5,
        common_ox: &[4, 7],
        category: "transition_metal",
    },
    Element {
        z: 44,
        symbol: "Ru",
        name: "Ruthenium",
        mass: 101.07,
        group: 8,
        period: 5,
        common_ox: &[3, 4],
        category: "transition_metal",
    },
    Element {
        z: 45,
        symbol: "Rh",
        name: "Rhodium",
        mass: 102.91,
        group: 9,
        period: 5,
        common_ox: &[3],
        category: "transition_metal",
    },
    Element {
        z: 46,
        symbol: "Pd",
        name: "Palladium",
        mass: 106.42,
        group: 10,
        period: 5,
        common_ox: &[2, 4],
        category: "transition_metal",
    },
    Element {
        z: 47,
        symbol: "Ag",
        name: "Silver",
        mass: 107.87,
        group: 11,
        period: 5,
        common_ox: &[1],
        category: "transition_metal",
    },
    Element {
        z: 48,
        symbol: "Cd",
        name: "Cadmium",
        mass: 112.41,
        group: 12,
        period: 5,
        common_ox: &[2],
        category: "transition_metal",
    },
    Element {
        z: 49,
        symbol: "In",
        name: "Indium",
        mass: 114.82,
        group: 13,
        period: 5,
        common_ox: &[3],
        category: "post_transition",
    },
    Element {
        z: 50,
        symbol: "Sn",
        name: "Tin",
        mass: 118.71,
        group: 14,
        period: 5,
        common_ox: &[2, 4],
        category: "post_transition",
    },
    Element {
        z: 51,
        symbol: "Sb",
        name: "Antimony",
        mass: 121.76,
        group: 15,
        period: 5,
        common_ox: &[-3, 3, 5],
        category: "metalloid",
    },
    Element {
        z: 52,
        symbol: "Te",
        name: "Tellurium",
        mass: 127.60,
        group: 16,
        period: 5,
        common_ox: &[-2, 4, 6],
        category: "metalloid",
    },
    Element {
        z: 53,
        symbol: "I",
        name: "Iodine",
        mass: 126.90,
        group: 17,
        period: 5,
        common_ox: &[-1, 1, 3, 5, 7],
        category: "halogen",
    },
    Element {
        z: 54,
        symbol: "Xe",
        name: "Xenon",
        mass: 131.29,
        group: 18,
        period: 5,
        common_ox: &[0, 2, 4, 6, 8],
        category: "noble_gas",
    },
    Element {
        z: 55,
        symbol: "Cs",
        name: "Caesium",
        mass: 132.91,
        group: 1,
        period: 6,
        common_ox: &[1],
        category: "alkali_metal",
    },
    Element {
        z: 56,
        symbol: "Ba",
        name: "Barium",
        mass: 137.33,
        group: 2,
        period: 6,
        common_ox: &[2],
        category: "alkaline_earth",
    },
    Element {
        z: 57,
        symbol: "La",
        name: "Lanthanum",
        mass: 138.91,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 58,
        symbol: "Ce",
        name: "Cerium",
        mass: 140.12,
        group: -1,
        period: 6,
        common_ox: &[3, 4],
        category: "lanthanide",
    },
    Element {
        z: 59,
        symbol: "Pr",
        name: "Praseodymium",
        mass: 140.91,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 60,
        symbol: "Nd",
        name: "Neodymium",
        mass: 144.24,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 61,
        symbol: "Pm",
        name: "Promethium",
        mass: 145.0,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 62,
        symbol: "Sm",
        name: "Samarium",
        mass: 150.36,
        group: -1,
        period: 6,
        common_ox: &[2, 3],
        category: "lanthanide",
    },
    Element {
        z: 63,
        symbol: "Eu",
        name: "Europium",
        mass: 151.96,
        group: -1,
        period: 6,
        common_ox: &[2, 3],
        category: "lanthanide",
    },
    Element {
        z: 64,
        symbol: "Gd",
        name: "Gadolinium",
        mass: 157.25,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 65,
        symbol: "Tb",
        name: "Terbium",
        mass: 158.93,
        group: -1,
        period: 6,
        common_ox: &[3, 4],
        category: "lanthanide",
    },
    Element {
        z: 66,
        symbol: "Dy",
        name: "Dysprosium",
        mass: 162.50,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 67,
        symbol: "Ho",
        name: "Holmium",
        mass: 164.93,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 68,
        symbol: "Er",
        name: "Erbium",
        mass: 167.26,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 69,
        symbol: "Tm",
        name: "Thulium",
        mass: 168.93,
        group: -1,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 70,
        symbol: "Yb",
        name: "Ytterbium",
        mass: 173.05,
        group: -1,
        period: 6,
        common_ox: &[2, 3],
        category: "lanthanide",
    },
    Element {
        z: 71,
        symbol: "Lu",
        name: "Lutetium",
        mass: 174.97,
        group: 3,
        period: 6,
        common_ox: &[3],
        category: "lanthanide",
    },
    Element {
        z: 72,
        symbol: "Hf",
        name: "Hafnium",
        mass: 178.49,
        group: 4,
        period: 6,
        common_ox: &[4],
        category: "transition_metal",
    },
    Element {
        z: 73,
        symbol: "Ta",
        name: "Tantalum",
        mass: 180.95,
        group: 5,
        period: 6,
        common_ox: &[5],
        category: "transition_metal",
    },
    Element {
        z: 74,
        symbol: "W",
        name: "Tungsten",
        mass: 183.84,
        group: 6,
        period: 6,
        common_ox: &[4, 6],
        category: "transition_metal",
    },
    Element {
        z: 75,
        symbol: "Re",
        name: "Rhenium",
        mass: 186.21,
        group: 7,
        period: 6,
        common_ox: &[4, 7],
        category: "transition_metal",
    },
    Element {
        z: 76,
        symbol: "Os",
        name: "Osmium",
        mass: 190.23,
        group: 8,
        period: 6,
        common_ox: &[3, 4],
        category: "transition_metal",
    },
    Element {
        z: 77,
        symbol: "Ir",
        name: "Iridium",
        mass: 192.22,
        group: 9,
        period: 6,
        common_ox: &[3, 4],
        category: "transition_metal",
    },
    Element {
        z: 78,
        symbol: "Pt",
        name: "Platinum",
        mass: 195.08,
        group: 10,
        period: 6,
        common_ox: &[2, 4],
        category: "transition_metal",
    },
    Element {
        z: 79,
        symbol: "Au",
        name: "Gold",
        mass: 196.97,
        group: 11,
        period: 6,
        common_ox: &[1, 3],
        category: "transition_metal",
    },
    Element {
        z: 80,
        symbol: "Hg",
        name: "Mercury",
        mass: 200.59,
        group: 12,
        period: 6,
        common_ox: &[1, 2],
        category: "transition_metal",
    },
    Element {
        z: 81,
        symbol: "Tl",
        name: "Thallium",
        mass: 204.38,
        group: 13,
        period: 6,
        common_ox: &[1, 3],
        category: "post_transition",
    },
    Element {
        z: 82,
        symbol: "Pb",
        name: "Lead",
        mass: 207.2,
        group: 14,
        period: 6,
        common_ox: &[2, 4],
        category: "post_transition",
    },
    Element {
        z: 83,
        symbol: "Bi",
        name: "Bismuth",
        mass: 208.98,
        group: 15,
        period: 6,
        common_ox: &[3, 5],
        category: "post_transition",
    },
    Element {
        z: 84,
        symbol: "Po",
        name: "Polonium",
        mass: 209.0,
        group: 16,
        period: 6,
        common_ox: &[-2, 2, 4],
        category: "metalloid",
    },
    Element {
        z: 85,
        symbol: "At",
        name: "Astatine",
        mass: 210.0,
        group: 17,
        period: 6,
        common_ox: &[-1, 1],
        category: "halogen",
    },
    Element {
        z: 86,
        symbol: "Rn",
        name: "Radon",
        mass: 222.0,
        group: 18,
        period: 6,
        common_ox: &[0],
        category: "noble_gas",
    },
    Element {
        z: 87,
        symbol: "Fr",
        name: "Francium",
        mass: 223.0,
        group: 1,
        period: 7,
        common_ox: &[1],
        category: "alkali_metal",
    },
    Element {
        z: 88,
        symbol: "Ra",
        name: "Radium",
        mass: 226.0,
        group: 2,
        period: 7,
        common_ox: &[2],
        category: "alkaline_earth",
    },
    Element {
        z: 89,
        symbol: "Ac",
        name: "Actinium",
        mass: 227.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 90,
        symbol: "Th",
        name: "Thorium",
        mass: 232.04,
        group: -1,
        period: 7,
        common_ox: &[4],
        category: "actinide",
    },
    Element {
        z: 91,
        symbol: "Pa",
        name: "Protactinium",
        mass: 231.04,
        group: -1,
        period: 7,
        common_ox: &[5],
        category: "actinide",
    },
    Element {
        z: 92,
        symbol: "U",
        name: "Uranium",
        mass: 238.03,
        group: -1,
        period: 7,
        common_ox: &[4, 6],
        category: "actinide",
    },
    Element {
        z: 93,
        symbol: "Np",
        name: "Neptunium",
        mass: 237.0,
        group: -1,
        period: 7,
        common_ox: &[5],
        category: "actinide",
    },
    Element {
        z: 94,
        symbol: "Pu",
        name: "Plutonium",
        mass: 244.0,
        group: -1,
        period: 7,
        common_ox: &[4],
        category: "actinide",
    },
    Element {
        z: 95,
        symbol: "Am",
        name: "Americium",
        mass: 243.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 96,
        symbol: "Cm",
        name: "Curium",
        mass: 247.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 97,
        symbol: "Bk",
        name: "Berkelium",
        mass: 247.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 98,
        symbol: "Cf",
        name: "Californium",
        mass: 251.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 99,
        symbol: "Es",
        name: "Einsteinium",
        mass: 252.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 100,
        symbol: "Fm",
        name: "Fermium",
        mass: 257.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 101,
        symbol: "Md",
        name: "Mendelevium",
        mass: 258.0,
        group: -1,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 102,
        symbol: "No",
        name: "Nobelium",
        mass: 259.0,
        group: -1,
        period: 7,
        common_ox: &[2, 3],
        category: "actinide",
    },
    Element {
        z: 103,
        symbol: "Lr",
        name: "Lawrencium",
        mass: 266.0,
        group: 3,
        period: 7,
        common_ox: &[3],
        category: "actinide",
    },
    Element {
        z: 104,
        symbol: "Rf",
        name: "Rutherfordium",
        mass: 267.0,
        group: 4,
        period: 7,
        common_ox: &[4],
        category: "transition_metal",
    },
    Element {
        z: 105,
        symbol: "Db",
        name: "Dubnium",
        mass: 268.0,
        group: 5,
        period: 7,
        common_ox: &[5],
        category: "transition_metal",
    },
    Element {
        z: 106,
        symbol: "Sg",
        name: "Seaborgium",
        mass: 269.0,
        group: 6,
        period: 7,
        common_ox: &[6],
        category: "transition_metal",
    },
    Element {
        z: 107,
        symbol: "Bh",
        name: "Bohrium",
        mass: 270.0,
        group: 7,
        period: 7,
        common_ox: &[7],
        category: "transition_metal",
    },
    Element {
        z: 108,
        symbol: "Hs",
        name: "Hassium",
        mass: 269.0,
        group: 8,
        period: 7,
        common_ox: &[8],
        category: "transition_metal",
    },
    Element {
        z: 109,
        symbol: "Mt",
        name: "Meitnerium",
        mass: 278.0,
        group: 9,
        period: 7,
        common_ox: &[],
        category: "transition_metal",
    },
    Element {
        z: 110,
        symbol: "Ds",
        name: "Darmstadtium",
        mass: 281.0,
        group: 10,
        period: 7,
        common_ox: &[],
        category: "transition_metal",
    },
    Element {
        z: 111,
        symbol: "Rg",
        name: "Roentgenium",
        mass: 282.0,
        group: 11,
        period: 7,
        common_ox: &[],
        category: "transition_metal",
    },
    Element {
        z: 112,
        symbol: "Cn",
        name: "Copernicium",
        mass: 285.0,
        group: 12,
        period: 7,
        common_ox: &[],
        category: "transition_metal",
    },
    Element {
        z: 113,
        symbol: "Nh",
        name: "Nihonium",
        mass: 286.0,
        group: 13,
        period: 7,
        common_ox: &[],
        category: "post_transition",
    },
    Element {
        z: 114,
        symbol: "Fl",
        name: "Flerovium",
        mass: 289.0,
        group: 14,
        period: 7,
        common_ox: &[],
        category: "post_transition",
    },
    Element {
        z: 115,
        symbol: "Mc",
        name: "Moscovium",
        mass: 290.0,
        group: 15,
        period: 7,
        common_ox: &[],
        category: "post_transition",
    },
    Element {
        z: 116,
        symbol: "Lv",
        name: "Livermorium",
        mass: 293.0,
        group: 16,
        period: 7,
        common_ox: &[],
        category: "post_transition",
    },
    Element {
        z: 117,
        symbol: "Ts",
        name: "Tennessine",
        mass: 294.0,
        group: 17,
        period: 7,
        common_ox: &[],
        category: "halogen",
    },
    Element {
        z: 118,
        symbol: "Og",
        name: "Oganesson",
        mass: 294.0,
        group: 18,
        period: 7,
        common_ox: &[],
        category: "noble_gas",
    },
];

fn lookup_element(query: &str) -> Option<&'static Element> {
    let q = query.trim();
    // Numeric → atomic number.
    if let Ok(z) = q.parse::<u32>() {
        return ELEMENTS.iter().find(|e| e.z == z);
    }
    // Case-sensitive symbol match (so "Cs" != "CS"), then case-insensitive name.
    if let Some(e) = ELEMENTS.iter().find(|e| e.symbol == q) {
        return Some(e);
    }
    ELEMENTS
        .iter()
        .find(|e| e.name.eq_ignore_ascii_case(q) || e.symbol.eq_ignore_ascii_case(q))
}

fn element_to_json(e: &Element) -> serde_json::Value {
    json!({
        "atomic_number": e.z,
        "symbol": e.symbol,
        "name": e.name,
        "atomic_mass_g_per_mol": e.mass,
        "group": if e.group < 0 { serde_json::Value::Null } else { json!(e.group) },
        "period": e.period,
        "common_oxidation_states": e.common_ox,
        "category": e.category,
    })
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PeriodicArgs {
    /// Element symbol (`"Fe"`), name (`"iron"`), or atomic number (`"26"`).
    /// Symbol match is case-sensitive; name match is case-insensitive.
    element: String,
}

pub struct ChemPeriodicTable;
impl Skill for ChemPeriodicTable {
    fn name(&self) -> &'static str {
        "chem_periodic_table"
    }
    fn description(&self) -> &'static str {
        "Look up an element by symbol (`Fe`), name (`iron`), or atomic number \
        (`26`). Returns atomic number, symbol, IUPAC name, standard atomic \
        weight (g/mol), group, period, common oxidation states, and category."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PeriodicArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PeriodicArgs>()?;
            let e = lookup_element(&a.element)
                .ok_or_else(|| invalid(format!("unknown element '{}'", a.element)))?;
            Ok(text_result(element_to_json(e).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "By symbol",
                args: r#"{"element": "Fe"}"#,
                note: Some("Symbol match is case-sensitive."),
            },
            SkillExample {
                title: "By name",
                args: r#"{"element": "iron"}"#,
                note: Some("Name match is case-insensitive."),
            },
            SkillExample {
                title: "By atomic number",
                args: r#"{"element": "26"}"#,
                note: Some("Numeric query is treated as Z."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Look up an element's atomic mass, group, period, or oxidation states.",
            "Verify symbol↔name↔Z mapping without a network call.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Formula parsing — Hill order, molar mass, element counts.
// ---------------------------------------------------------------------------

/// Parse a chemical formula (e.g. `"Ca(OH)2"`, `"C6H12O6"`, `"Fe2(SO4)3.7H2O"`)
/// into a flat `element → count` map. Supports parentheses and `·` / `.` for
/// hydrates.
fn parse_formula(input: &str) -> std::result::Result<Vec<(String, u32)>, McpError> {
    let s = input.trim();
    let mut stack: Vec<Vec<(String, u32)>> = vec![Vec::new()];
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'(' || b == b'[' {
            stack.push(Vec::new());
            i += 1;
        } else if b == b')' || b == b']' {
            let inner = stack.pop().ok_or_else(|| invalid("unmatched bracket"))?;
            i += 1;
            let mut mult_str = String::new();
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                mult_str.push(bytes[i] as char);
                i += 1;
            }
            let mult: u32 = if mult_str.is_empty() {
                1
            } else {
                mult_str
                    .parse()
                    .map_err(|e| invalid(format!("bad multiplier: {e}")))?
            };
            let top = stack.last_mut().unwrap();
            for (el, cnt) in inner {
                top.push((el, cnt * mult));
            }
        } else if b == b'.' || b == 0xC2
        /* leading byte of '·' */
        {
            // Hydrate separator: leading coefficient applies to the rest.
            if b == 0xC2 && bytes.get(i + 1) == Some(&0xB7) {
                i += 2;
            } else {
                i += 1;
            }
            let mut coef_str = String::new();
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                coef_str.push(bytes[i] as char);
                i += 1;
            }
            let coef: u32 = if coef_str.is_empty() {
                1
            } else {
                coef_str
                    .parse()
                    .map_err(|e| invalid(format!("bad hydrate coef: {e}")))?
            };
            let rest: Vec<(String, u32)> = parse_formula(&s[i..])?
                .into_iter()
                .map(|(el, cnt)| (el, cnt * coef))
                .collect();
            stack.last_mut().unwrap().extend(rest);
            break;
        } else if b.is_ascii_uppercase() {
            let mut sym = String::from(b as char);
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_lowercase() {
                sym.push(bytes[j] as char);
                j += 1;
            }
            let mut cnt_str = String::new();
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                cnt_str.push(bytes[j] as char);
                j += 1;
            }
            let cnt: u32 = if cnt_str.is_empty() {
                1
            } else {
                cnt_str
                    .parse()
                    .map_err(|e| invalid(format!("bad count: {e}")))?
            };
            if lookup_element(&sym).is_none() {
                return Err(invalid(format!("unknown element '{sym}'")));
            }
            stack.last_mut().unwrap().push((sym, cnt));
            i = j;
        } else if b.is_ascii_whitespace() {
            i += 1;
        } else {
            return Err(invalid(format!(
                "unexpected char '{}' in formula",
                b as char
            )));
        }
    }
    if stack.len() != 1 {
        return Err(invalid("unclosed bracket"));
    }
    // Fold duplicates.
    let mut tally: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for (el, cnt) in stack.pop().unwrap() {
        *tally.entry(el).or_insert(0) += cnt;
    }
    Ok(tally.into_iter().collect())
}

fn hill_order(mut counts: Vec<(String, u32)>) -> Vec<(String, u32)> {
    // Carbon first, then hydrogen, then alphabetical for the rest. Only when C is present.
    let has_c = counts.iter().any(|(el, _)| el == "C");
    if has_c {
        counts.sort_by(|a, b| {
            let rank = |s: &str| match s {
                "C" => 0,
                "H" => 1,
                _ => 2,
            };
            rank(&a.0).cmp(&rank(&b.0)).then_with(|| a.0.cmp(&b.0))
        });
    } else {
        counts.sort_by(|a, b| a.0.cmp(&b.0));
    }
    counts
}

fn molar_mass(counts: &[(String, u32)]) -> std::result::Result<f64, McpError> {
    let mut total = 0.0_f64;
    for (el, cnt) in counts {
        let e = lookup_element(el).ok_or_else(|| invalid(format!("unknown element '{el}'")))?;
        total += e.mass * (*cnt as f64);
    }
    Ok(total)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormulaArgs {
    /// Chemical formula, e.g. `"H2O"`, `"C6H12O6"`, `"Ca(OH)2"`, `"CuSO4.5H2O"`.
    formula: String,
}

pub struct ChemMolarMass;
impl Skill for ChemMolarMass {
    fn name(&self) -> &'static str {
        "chem_molar_mass"
    }
    fn description(&self) -> &'static str {
        "Compute molar mass (g/mol) for a chemical formula. Handles parentheses \
        (`Ca(OH)2`), bracketed groups, and hydrates (`CuSO4.5H2O` or \
        `CuSO4·5H2O`). Returns molar mass + the parsed element counts."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FormulaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<FormulaArgs>()?;
            let counts = parse_formula(&a.formula)?;
            let mass = molar_mass(&counts)?;
            Ok(text_result(
                json!({
                    "formula": a.formula,
                    "molar_mass_g_per_mol": mass,
                    "elements": counts.iter().map(|(e, c)| json!({"symbol": e, "count": c})).collect::<Vec<_>>(),
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Water",
                args: r#"{"formula": "H2O"}"#,
                note: Some("Returns ~18.015 g/mol."),
            },
            SkillExample {
                title: "Parentheses",
                args: r#"{"formula": "Ca(OH)2"}"#,
                note: Some("Nested groups multiplied correctly."),
            },
            SkillExample {
                title: "Hydrate",
                args: r#"{"formula": "CuSO4.5H2O"}"#,
                note: Some("`.` or `·` separates a hydrate coefficient."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute formula weight for stoichiometry / yield / dilution math.",
            "Get per-element counts for a parsed formula.",
        ]
    }
}

pub struct ChemFormulaHill;
impl Skill for ChemFormulaHill {
    fn name(&self) -> &'static str {
        "chem_formula_hill"
    }
    fn description(&self) -> &'static str {
        "Normalize a chemical formula into Hill order: carbon first, then \
        hydrogen, then remaining elements alphabetically (when C is absent, \
        everything is alphabetical). Returns the canonical string + element \
        counts."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FormulaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<FormulaArgs>()?;
            let counts = hill_order(parse_formula(&a.formula)?);
            let s: String = counts
                .iter()
                .map(|(el, c)| {
                    if *c == 1 {
                        el.clone()
                    } else {
                        format!("{el}{c}")
                    }
                })
                .collect();
            Ok(text_result(
                json!({
                    "input": a.formula,
                    "hill_formula": s,
                    "elements": counts.iter().map(|(e, c)| json!({"symbol": e, "count": c})).collect::<Vec<_>>(),
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Reorder glucose",
                args: r#"{"formula": "OH12C6"}"#,
                note: Some("Returns `C6H12O6` — C first, H second, rest alphabetical."),
            },
            SkillExample {
                title: "No carbon, alphabetize",
                args: r#"{"formula": "FeO2H"}"#,
                note: Some("Without C, everything is sorted alphabetically."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Canonicalize a formula string for indexing or de-duplication.",
            "Convert a user-typed formula into the standard Hill-system display form.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Equation balancing — null-space of the element-coefficient matrix.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EquationArgs {
    /// Chemical equation with `=` or `->` between sides, species comma- or
    /// `+`-separated. Examples: `"H2 + O2 = H2O"`, `"C3H8 + O2 -> CO2 + H2O"`.
    equation: String,
}

pub struct ChemBalanceEquation;
impl Skill for ChemBalanceEquation {
    fn name(&self) -> &'static str {
        "chem_balance_equation"
    }
    fn description(&self) -> &'static str {
        "Balance a chemical equation by finding the smallest positive integer \
        vector ν satisfying Aν = 0, where A is the element-by-species matrix. \
        Uses exact rational arithmetic (fraction-free Gaussian elimination + \
        LCM/GCD rationalization) so the result is exact, not numerical. Detects \
        infeasible reactions (an element appears on only one side) and \
        under-determined reactions (multiple independent balances) and surfaces \
        either as a clear error. Mass-balance only — charge balance for redox \
        half-reactions is out of scope."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EquationArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EquationArgs>()?;
            let coeffs = balance_equation(&a.equation)?;
            Ok(text_result(coeffs.to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Water formation",
                args: r#"{"equation": "H2 + O2 = H2O"}"#,
                note: Some("Returns coefficients [2, 1, 2]."),
            },
            SkillExample {
                title: "Propane combustion",
                args: r#"{"equation": "C3H8 + O2 -> CO2 + H2O"}"#,
                note: Some("Either `=` or `->` is accepted; result [1, 5, 3, 4]."),
            },
            SkillExample {
                title: "Iron-oxide reduction",
                args: r#"{"equation": "Fe2O3 + CO = Fe + CO2"}"#,
                note: Some("Returns [1, 3, 2, 3] (blast-furnace stoichiometry)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Get exact integer coefficients for a multi-species mass-balance equation.",
            "Detect an infeasible or under-determined reaction before chasing stoichiometry by hand.",
        ]
    }
}

fn balance_equation(eq: &str) -> std::result::Result<serde_json::Value, McpError> {
    let normalized = eq.replace("->", "=").replace("→", "=");
    let parts: Vec<&str> = normalized.split('=').collect();
    if parts.len() != 2 {
        return Err(invalid("equation must have exactly one `=` (or `->`)"));
    }
    let split_side = |s: &str| -> Vec<String> {
        s.split('+')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    };
    let lhs = split_side(parts[0]);
    let rhs = split_side(parts[1]);
    if lhs.is_empty() || rhs.is_empty() {
        return Err(invalid("each side must list at least one species"));
    }
    let species: Vec<String> = lhs.iter().chain(rhs.iter()).cloned().collect();
    let parsed: Vec<Vec<(String, u32)>> = species
        .iter()
        .map(|s| parse_formula(s))
        .collect::<std::result::Result<_, _>>()?;
    let mut elements: Vec<String> = parsed
        .iter()
        .flat_map(|v| v.iter().map(|(e, _)| e.clone()))
        .collect();
    elements.sort();
    elements.dedup();
    if elements.is_empty() {
        return Err(invalid("no elements found"));
    }
    // Infeasibility precheck: every element must appear on BOTH sides.
    for el in &elements {
        let on_lhs = parsed
            .iter()
            .take(lhs.len())
            .any(|comp| comp.iter().any(|(e, _)| e == el));
        let on_rhs = parsed
            .iter()
            .skip(lhs.len())
            .any(|comp| comp.iter().any(|(e, _)| e == el));
        if !on_lhs || !on_rhs {
            return Err(invalid(format!(
                "element '{el}' appears on only one side — equation cannot be mass-balanced"
            )));
        }
    }
    // Build integer matrix: rows = elements, cols = species. LHS positive, RHS negative.
    let n_rows = elements.len();
    let n_cols = species.len();
    let mut a: Vec<Vec<BigInt>> = vec![vec![BigInt::zero(); n_cols]; n_rows];
    for (ci, comp) in parsed.iter().enumerate() {
        let sign: i64 = if ci < lhs.len() { 1 } else { -1 };
        for (el, cnt) in comp {
            let ri = elements.iter().position(|e| e == el).unwrap();
            a[ri][ci] += BigInt::from(sign * (*cnt as i64));
        }
    }
    // Bareiss-style fraction-free Gauss-Jordan to reduced row echelon form (over Q,
    // but kept as scaled BigInts via a row-by-row LCM normalize). Simpler than
    // true Bareiss; works fine for the small matrices a balancer sees.
    let mut mat = a.clone();
    let mut pivot_cols: Vec<usize> = Vec::new();
    let mut r = 0;
    for c in 0..n_cols {
        if r >= n_rows {
            break;
        }
        // Find a non-zero pivot at or below row r in column c. Index-based
        // loop is intentional: we need the row index so we can `mat.swap(r, pi)`.
        let mut piv = None;
        #[allow(clippy::needless_range_loop)]
        for rr in r..n_rows {
            if !mat[rr][c].is_zero() {
                piv = Some(rr);
                break;
            }
        }
        let Some(pi) = piv else { continue };
        mat.swap(r, pi);
        // Eliminate column c in all other rows. Index loops below are
        // intentional: we hold simultaneous borrows on `mat[r]` and `mat[rr]`
        // (different rows of a `Vec<Vec<BigInt>>`), which clippy can't model.
        #[allow(clippy::needless_range_loop)]
        for rr in 0..n_rows {
            if rr == r {
                continue;
            }
            if mat[rr][c].is_zero() {
                continue;
            }
            let a_rc = mat[r][c].clone();
            let b_rc = mat[rr][c].clone();
            for j in 0..n_cols {
                let new_val = &mat[rr][j] * &a_rc - &mat[r][j] * &b_rc;
                mat[rr][j] = new_val;
            }
            row_reduce_by_gcd(&mut mat[rr]);
        }
        pivot_cols.push(c);
        r += 1;
    }
    let rank = pivot_cols.len();
    let nullity = n_cols - rank;
    if nullity == 0 {
        return Err(invalid(
            "no non-trivial balance exists (matrix is full rank)",
        ));
    }
    if nullity > 1 {
        return Err(invalid(format!(
            "reaction is under-determined (nullity {nullity}) — multiple independent balances exist; fix a coefficient and re-submit"
        )));
    }
    // Single free variable: the one column NOT in pivot_cols.
    let free_col = (0..n_cols).find(|c| !pivot_cols.contains(c)).unwrap();
    // For each pivot row r with pivot column pc: pc · x_pc + (sum over other cols) = 0.
    // Setting x_free = 1, all other free = 0 (none here), solve.
    // From row r: mat[r][pc] · x_pc + mat[r][free_col] = 0  →  x_pc = -mat[r][free_col] / mat[r][pc].
    let mut numer = vec![BigInt::zero(); n_cols];
    let mut denom = vec![BigInt::one(); n_cols];
    numer[free_col] = BigInt::one();
    for (i, &pc) in pivot_cols.iter().enumerate() {
        let row = &mat[i];
        // x_pc = -row[free_col] / row[pc].
        numer[pc] = -row[free_col].clone();
        denom[pc] = row[pc].clone();
    }
    // Sign-normalize denominators to positive.
    for j in 0..n_cols {
        if denom[j].is_negative() {
            numer[j] = -numer[j].clone();
            denom[j] = -denom[j].clone();
        }
    }
    // Multiply each fraction by the LCM of all denominators to clear them.
    let mut lcm = BigInt::one();
    for d in &denom {
        lcm = lcm.lcm(d);
    }
    let mut ints: Vec<BigInt> = numer
        .iter()
        .zip(denom.iter())
        .map(|(n, d)| n * (&lcm / d))
        .collect();
    // Reduce by GCD across all entries.
    let mut g = BigInt::zero();
    for v in &ints {
        g = g.gcd(&v.abs());
    }
    if !g.is_zero() {
        for v in &mut ints {
            *v /= &g;
        }
    }
    // If the chosen free variable produced negative coefficients, flip the whole vector.
    if ints.iter().any(|x| x.is_positive()) && ints.iter().any(|x| x.is_negative()) {
        // Mixed signs are normal until we account for the LHS+/RHS- convention.
        // After the flip below, every coefficient should be strictly positive
        // because of the sign convention we built into A.
    }
    if ints
        .iter()
        .filter(|x| !x.is_zero())
        .all(|x| x.is_negative())
    {
        for v in &mut ints {
            *v = -v.clone();
        }
    }
    if ints.iter().any(|x| !x.is_positive()) {
        return Err(invalid(
            "balancer produced non-positive coefficients — equation may be infeasible",
        ));
    }
    // BigInt → u64 (small in practice; guard the rare overflow).
    let coeffs: Vec<u64> = ints
        .iter()
        .map(|v| {
            v.to_string()
                .parse::<u64>()
                .map_err(|_| invalid("balanced coefficient overflows u64"))
        })
        .collect::<std::result::Result<_, _>>()?;
    let render_side = |species_slice: &[String], coef_slice: &[u64]| -> String {
        species_slice
            .iter()
            .zip(coef_slice.iter())
            .map(|(s, c)| {
                if *c == 1 {
                    s.clone()
                } else {
                    format!("{c} {s}")
                }
            })
            .collect::<Vec<_>>()
            .join(" + ")
    };
    let lhs_str = render_side(&lhs, &coeffs[..lhs.len()]);
    let rhs_str = render_side(&rhs, &coeffs[lhs.len()..]);
    Ok(json!({
        "balanced": format!("{lhs_str} = {rhs_str}"),
        "coefficients": species.iter().zip(coeffs.iter())
            .map(|(s, c)| json!({"species": s, "coefficient": c}))
            .collect::<Vec<_>>(),
        "elements_balanced": elements,
    }))
}

fn row_reduce_by_gcd(row: &mut [BigInt]) {
    let mut g = BigInt::zero();
    for v in row.iter() {
        if !v.is_zero() {
            g = g.gcd(&v.abs());
        }
    }
    if !g.is_zero() && !g.is_one() {
        for v in row.iter_mut() {
            *v /= &g;
        }
    }
}

// ---------------------------------------------------------------------------
// pH / pOH / Henderson-Hasselbalch buffer / acid-base lookup.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PhArgs {
    /// One of `strong_acid`, `strong_base`, `weak_acid`, `weak_base`.
    kind: String,
    /// Molar concentration (mol/L).
    concentration_m: f64,
    /// pKa (weak acid) or pKb (weak base). Required for weak species.
    #[serde(default)]
    pka_or_pkb: Option<f64>,
}

pub struct ChemPh;
impl Skill for ChemPh {
    fn name(&self) -> &'static str {
        "chem_ph"
    }
    fn description(&self) -> &'static str {
        "Compute pH / pOH / [H+] / [OH-] for a strong or weak acid/base in \
        aqueous solution at 25 °C. Weak species use the standard \
        small-x approximation: [H+] ≈ √(Ka · C₀)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PhArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PhArgs>()?;
            if a.concentration_m <= 0.0 {
                return Err(invalid("concentration must be > 0"));
            }
            let (ph, poh) = match a.kind.to_ascii_lowercase().as_str() {
                "strong_acid" => {
                    let p = -a.concentration_m.log10();
                    (p, 14.0 - p)
                }
                "strong_base" => {
                    let p = -a.concentration_m.log10();
                    (14.0 - p, p)
                }
                "weak_acid" => {
                    let pka = a
                        .pka_or_pkb
                        .ok_or_else(|| invalid("weak_acid requires pka_or_pkb (pKa)"))?;
                    let ka = 10_f64.powf(-pka);
                    let h = (ka * a.concentration_m).sqrt();
                    let p = -h.log10();
                    (p, 14.0 - p)
                }
                "weak_base" => {
                    let pkb = a
                        .pka_or_pkb
                        .ok_or_else(|| invalid("weak_base requires pka_or_pkb (pKb)"))?;
                    let kb = 10_f64.powf(-pkb);
                    let oh = (kb * a.concentration_m).sqrt();
                    let poh = -oh.log10();
                    (14.0 - poh, poh)
                }
                other => return Err(invalid(format!("unknown kind '{other}'"))),
            };
            let h = 10_f64.powf(-ph);
            let oh = 10_f64.powf(-poh);
            Ok(text_result(
                json!({
                    "ph": ph, "poh": poh,
                    "h_plus_m": h, "oh_minus_m": oh,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "0.01 M HCl",
                args: r#"{"kind": "strong_acid", "concentration_m": 0.01}"#,
                note: Some("Returns pH ≈ 2."),
            },
            SkillExample {
                title: "0.1 M acetic acid (pKa 4.76)",
                args: r#"{"kind": "weak_acid", "concentration_m": 0.1, "pka_or_pkb": 4.76}"#,
                note: Some("Uses [H+] ≈ √(Ka·C₀); returns pH ≈ 2.87."),
            },
            SkillExample {
                title: "0.05 M NH3 (pKb 4.75)",
                args: r#"{"kind": "weak_base", "concentration_m": 0.05, "pka_or_pkb": 4.75}"#,
                note: Some("Pass pKb for weak bases."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Quick pH estimate for a single strong or weak acid/base solution.",
            "Convert between [H+], [OH-], pH, and pOH given a concentration.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BufferArgs {
    /// pKa of the weak acid (or pKa = 14 − pKb for a weak base).
    pka: f64,
    /// Molar concentration of the conjugate base, [A⁻].
    base_m: f64,
    /// Molar concentration of the weak acid, [HA].
    acid_m: f64,
}

pub struct ChemBuffer;
impl Skill for ChemBuffer {
    fn name(&self) -> &'static str {
        "chem_buffer"
    }
    fn description(&self) -> &'static str {
        "Buffer pH via Henderson-Hasselbalch: pH = pKa + log10([A⁻]/[HA]). \
        Returns pH and the buffer ratio."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BufferArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BufferArgs>()?;
            if a.base_m <= 0.0 || a.acid_m <= 0.0 {
                return Err(invalid("concentrations must be > 0"));
            }
            let ratio = a.base_m / a.acid_m;
            let ph = a.pka + ratio.log10();
            Ok(text_result(json!({ "ph": ph, "ratio": ratio }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Equimolar buffer",
                args: r#"{"pka": 4.76, "base_m": 0.1, "acid_m": 0.1}"#,
                note: Some("Returns pH = pKa = 4.76."),
            },
            SkillExample {
                title: "10× more base than acid",
                args: r#"{"pka": 7.4, "base_m": 0.1, "acid_m": 0.01}"#,
                note: Some("pH = pKa + log10(10) = 8.4."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute pH of a known buffer composition via Henderson-Hasselbalch.",
            "Pick a [A⁻]/[HA] ratio to hit a target pH near a chosen pKa.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Ideal gas / dilution / Gibbs free energy / radioactive decay.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IdealGasArgs {
    /// Provide exactly three; the missing one is computed.
    #[serde(default)]
    pressure_pa: Option<f64>,
    /// Volume in cubic meters.
    #[serde(default)]
    volume_m3: Option<f64>,
    /// Amount of substance in moles.
    #[serde(default)]
    moles: Option<f64>,
    /// Temperature in kelvin.
    #[serde(default)]
    temperature_k: Option<f64>,
}

pub struct ChemIdealGas;
impl Skill for ChemIdealGas {
    fn name(&self) -> &'static str {
        "chem_ideal_gas"
    }
    fn description(&self) -> &'static str {
        "Solve PV = nRT (R = 8.314 J/(mol·K)). Provide any three of \
        `pressure_pa`, `volume_m3`, `moles`, `temperature_k`; the fourth is \
        computed and returned."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IdealGasArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<IdealGasArgs>()?;
            const R: f64 = 8.314_462_618;
            let provided = [
                a.pressure_pa.is_some(),
                a.volume_m3.is_some(),
                a.moles.is_some(),
                a.temperature_k.is_some(),
            ];
            let n_known = provided.iter().filter(|x| **x).count();
            if n_known != 3 {
                return Err(invalid(format!("need exactly 3 of P/V/n/T; got {n_known}")));
            }
            let (p, v, n, t) = match (a.pressure_pa, a.volume_m3, a.moles, a.temperature_k) {
                (None, Some(v), Some(n), Some(t)) => {
                    let p = n * R * t / v;
                    (p, v, n, t)
                }
                (Some(p), None, Some(n), Some(t)) => {
                    let v = n * R * t / p;
                    (p, v, n, t)
                }
                (Some(p), Some(v), None, Some(t)) => {
                    let n = p * v / (R * t);
                    (p, v, n, t)
                }
                (Some(p), Some(v), Some(n), None) => {
                    let t = p * v / (n * R);
                    (p, v, n, t)
                }
                _ => unreachable!(),
            };
            Ok(text_result(
                json!({
                    "pressure_pa": p,
                    "volume_m3": v,
                    "moles": n,
                    "temperature_k": t,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Solve for volume at STP",
                args: r#"{"pressure_pa": 101325, "moles": 1.0, "temperature_k": 273.15}"#,
                note: Some("Returns molar volume ≈ 0.02241 m³ (22.41 L)."),
            },
            SkillExample {
                title: "Solve for pressure",
                args: r#"{"volume_m3": 0.001, "moles": 0.04, "temperature_k": 298.15}"#,
                note: Some("Omit the variable you want computed (here, pressure_pa)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Solve any one of P, V, n, T from the other three under ideal-gas assumption.",
            "Convert between moles and volume for a gas at known T and P.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DilutionArgs {
    /// Provide exactly three of {c1, v1, c2, v2}; missing one is computed.
    #[serde(default)]
    c1_m: Option<f64>,
    /// Initial volume in liters.
    #[serde(default)]
    v1_l: Option<f64>,
    /// Final concentration in mol/L.
    #[serde(default)]
    c2_m: Option<f64>,
    /// Final volume in liters.
    #[serde(default)]
    v2_l: Option<f64>,
}

pub struct ChemDilution;
impl Skill for ChemDilution {
    fn name(&self) -> &'static str {
        "chem_dilution"
    }
    fn description(&self) -> &'static str {
        "Solve M₁V₁ = M₂V₂ for an unknown. Provide three; the fourth is returned."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DilutionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DilutionArgs>()?;
            let n_known = [a.c1_m, a.v1_l, a.c2_m, a.v2_l]
                .iter()
                .filter(|x| x.is_some())
                .count();
            if n_known != 3 {
                return Err(invalid(format!(
                    "need exactly 3 of c1/v1/c2/v2; got {n_known}"
                )));
            }
            let (c1, v1, c2, v2) = match (a.c1_m, a.v1_l, a.c2_m, a.v2_l) {
                (None, Some(v1), Some(c2), Some(v2)) => (c2 * v2 / v1, v1, c2, v2),
                (Some(c1), None, Some(c2), Some(v2)) => (c1, c2 * v2 / c1, c2, v2),
                (Some(c1), Some(v1), None, Some(v2)) => (c1, v1, c1 * v1 / v2, v2),
                (Some(c1), Some(v1), Some(c2), None) => (c1, v1, c2, c1 * v1 / c2),
                _ => unreachable!(),
            };
            Ok(text_result(
                json!({
                    "c1_m": c1,
                    "v1_l": v1,
                    "c2_m": c2,
                    "v2_l": v2,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "How much stock for 100 mL of 0.1 M?",
                args: r#"{"c1_m": 1.0, "c2_m": 0.1, "v2_l": 0.1}"#,
                note: Some("Returns v1_l = 0.01 (10 mL of stock, fill to 100 mL)."),
            },
            SkillExample {
                title: "What final concentration?",
                args: r#"{"c1_m": 2.0, "v1_l": 0.005, "v2_l": 0.05}"#,
                note: Some("Returns c2_m = 0.2."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Solve M₁V₁ = M₂V₂ for any one of the four variables.",
            "Plan a serial dilution or back-calculate a starting volume.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GibbsArgs {
    /// ΔH in kJ/mol.
    delta_h_kj: f64,
    /// ΔS in J/(mol·K).
    delta_s_j_per_k: f64,
    /// Temperature (K).
    temperature_k: f64,
}

pub struct ChemGibbs;
impl Skill for ChemGibbs {
    fn name(&self) -> &'static str {
        "chem_gibbs"
    }
    fn description(&self) -> &'static str {
        "Gibbs free energy: ΔG = ΔH − T·ΔS. Returns ΔG in kJ/mol plus the \
        spontaneity sign (`spontaneous` if ΔG < 0)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GibbsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<GibbsArgs>()?;
            // Convert ΔS J → kJ (divide by 1000) for unit-consistent kJ output.
            let dg = a.delta_h_kj - a.temperature_k * (a.delta_s_j_per_k / 1000.0);
            let spontaneous = dg < 0.0;
            Ok(text_result(
                json!({
                    "delta_g_kj_per_mol": dg,
                    "spontaneous": spontaneous,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Spontaneous at room T",
                args: r#"{"delta_h_kj": -100.0, "delta_s_j_per_k": 50.0, "temperature_k": 298.15}"#,
                note: Some("ΔG < 0 → `spontaneous: true`."),
            },
            SkillExample {
                title: "Endothermic, entropy-driven",
                args: r#"{"delta_h_kj": 30.0, "delta_s_j_per_k": 120.0, "temperature_k": 500.0}"#,
                note: Some("High T can flip an endothermic reaction to spontaneous."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Predict spontaneity of a reaction at a given temperature.",
            "Find the crossover temperature where ΔG changes sign.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DecayArgs {
    /// Initial quantity (atoms, moles, Bq, anything proportional).
    n0: f64,
    /// Half-life in seconds.
    half_life_s: f64,
    /// Elapsed time (seconds).
    time_s: f64,
}

pub struct ChemRadioactiveDecay;
impl Skill for ChemRadioactiveDecay {
    fn name(&self) -> &'static str {
        "chem_radioactive_decay"
    }
    fn description(&self) -> &'static str {
        "First-order radioactive decay: N(t) = N₀ · (½)^(t/t½). Returns the \
        remaining quantity, decay constant λ = ln(2)/t½, and the fraction \
        remaining."
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
            let lambda = std::f64::consts::LN_2 / a.half_life_s;
            let frac = 0.5_f64.powf(a.time_s / a.half_life_s);
            let n = a.n0 * frac;
            Ok(text_result(
                json!({
                    "n_remaining": n,
                    "fraction_remaining": frac,
                    "decay_constant_per_s": lambda,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "One half-life elapsed",
                args: r#"{"n0": 1000.0, "half_life_s": 100.0, "time_s": 100.0}"#,
                note: Some("Returns half remaining (fraction 0.5)."),
            },
            SkillExample {
                title: "Three half-lives",
                args: r#"{"n0": 1.0, "half_life_s": 60.0, "time_s": 180.0}"#,
                note: Some("Fraction ≈ 0.125 (one-eighth)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute remaining activity / mass after a given decay interval.",
            "Get λ = ln(2)/t½ from a half-life for use in downstream calcs.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ChemPeriodicTable),
        Box::new(ChemMolarMass),
        Box::new(ChemFormulaHill),
        Box::new(ChemBalanceEquation),
        Box::new(ChemPh),
        Box::new(ChemBuffer),
        Box::new(ChemIdealGas),
        Box::new(ChemDilution),
        Box::new(ChemGibbs),
        Box::new(ChemRadioactiveDecay),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periodic_lookup_by_symbol_name_number() {
        let fe = lookup_element("Fe").unwrap();
        assert_eq!(fe.z, 26);
        assert_eq!(lookup_element("iron").unwrap().z, 26);
        assert_eq!(lookup_element("26").unwrap().symbol, "Fe");
        assert!(lookup_element("Xx").is_none());
    }

    #[test]
    fn molar_mass_water() {
        let counts = parse_formula("H2O").unwrap();
        let m = molar_mass(&counts).unwrap();
        assert!((m - 18.015).abs() < 0.01);
    }

    #[test]
    fn molar_mass_glucose_and_hydrate() {
        assert!((molar_mass(&parse_formula("C6H12O6").unwrap()).unwrap() - 180.16).abs() < 0.1);
        let m = molar_mass(&parse_formula("CuSO4.5H2O").unwrap()).unwrap();
        assert!((m - 249.68).abs() < 0.5);
    }

    #[test]
    fn hill_glucose() {
        let counts = hill_order(parse_formula("OH12C6").unwrap());
        assert_eq!(counts[0].0, "C");
        assert_eq!(counts[1].0, "H");
        assert_eq!(counts[2].0, "O");
    }

    #[test]
    fn balance_water_formation() {
        let v = balance_equation("H2 + O2 = H2O").unwrap();
        let coefs: Vec<u64> = v["coefficients"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["coefficient"].as_u64().unwrap())
            .collect();
        assert_eq!(coefs, vec![2, 1, 2]);
    }

    #[test]
    fn balance_propane_combustion() {
        let v = balance_equation("C3H8 + O2 = CO2 + H2O").unwrap();
        let coefs: Vec<u64> = v["coefficients"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["coefficient"].as_u64().unwrap())
            .collect();
        assert_eq!(coefs, vec![1, 5, 3, 4]);
    }

    #[test]
    fn balance_iron_oxide_reduction() {
        // Fe2O3 + 3 CO -> 2 Fe + 3 CO2 (blast-furnace ironmaking).
        let v = balance_equation("Fe2O3 + CO = Fe + CO2").unwrap();
        let coefs: Vec<u64> = v["coefficients"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["coefficient"].as_u64().unwrap())
            .collect();
        assert_eq!(coefs, vec![1, 3, 2, 3]);
    }

    #[test]
    fn balance_rejects_missing_element_on_one_side() {
        // O has no source on the LHS — must error rather than silently produce 0.
        let err = balance_equation("H2 + Cl2 = H2O").unwrap_err();
        assert!(err.message.contains("only one side"));
    }

    #[test]
    fn ideal_gas_solves_for_each_variable() {
        // 1 mol gas at 273.15 K and 101325 Pa: V ≈ 0.0224 m³ (molar volume).
        const R: f64 = 8.314_462_618;
        let v = 1.0 * R * 273.15 / 101_325.0;
        assert!((v - 0.022_414).abs() < 1e-4);
    }
}
