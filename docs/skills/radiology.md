# Radiation protection / health physics — `rad_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/radiology.rs`](../../src/skills/radiology.rs) |
| **Tools** | `rad_isotope_lookup`, `rad_units`, `rad_attenuation`, `rad_inverse_square`, `rad_dose_rate`, `rad_equivalent_dose`, `rad_effective_half_life`, `rad_occupational_limits`, `rad_shielding_thickness`, `rad_alara` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

General-purpose radioactivity / dose math for industrial, research, and
calibration contexts — not specifically medical imaging. Covers dose-unit
conversion, ICRP 103 radiation weighting, exponential attenuation (with
mass-attenuation tables for Pb / concrete / steel / water / Al at common
γ energies), inverse-square, dose rate from a vendored Γ table, biokinetic
half-life, ALARA time/distance/shielding, and the two main occupational-
limit regimes.

## Source citations

- **Dose-unit definitions**: ICRU Report 85 (2011). 1 Gy = 1 J/kg;
  1 rad = 0.01 Gy; 1 Sv = 1 J/kg; 1 rem = 0.01 Sv; 1 R = 2.58e-4 C/kg
  (NCRP 82, exact).
- **W/e for dry air**: 33.97 J/C (NIST / AAPM TG-21 & TG-51). Implies
  K_air [Gy] = X [R] · 8.76e-3.
- **Radiation weighting factors w_R**: ICRP Publication 103 (2007),
  Annex B. Photons / electrons / muons w_R = 1; protons & charged pions
  w_R = 2; α, fission fragments, heavy ions w_R = 20; neutrons via the
  piecewise continuous function Eq. (B.1.1).
- **Specific air-kerma rate constants Γ**: Unger & Trubey,
  ORNL/RSIC-45/R1 (1982); NCRP 151 Appendix A.
- **Isotope data**: NNDC NuDat 3.0 (<https://www.nndc.bnl.gov/nudat3/>),
  IAEA Live Chart.
- **Mass attenuation coefficients μ/ρ**: NIST XCOM
  (<https://physics.nist.gov/PhysRefData/Xcom/>), interpolated log-log
  between 100–2000 keV anchor points.
- **Annual occupational limits**: ICRP 103 (intl.) and 10 CFR 20 /
  NCRP 116 (US). ICRP 118 (2011) cut the lens-of-eye limit to 20 mSv/y —
  the US NRC retains 150 mSv/y.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `rad_isotope_lookup` | `isotope` (`Co-60`, `Cs137`, `Ir-192`, …) | Half-life, decay mode, primary photons, Γ at 1 m. |
| `rad_units` | `direction`, `value` | Gy ↔ rad; Sv ↔ rem; R ↔ Gy_air. |
| `rad_attenuation` | `mu`, `thickness`, `i0?` | Beer-Lambert I(x), HVL, TVL. |
| `rad_inverse_square` | `d_ref`, `r_ref`, `r_target` | D(r) = D_ref · (r_ref/r)². |
| `rad_dose_rate` | `isotope`, `activity_gbq`, `distance_m` | Idealized point-source dose rate from vendored Γ. |
| `rad_equivalent_dose` | `d_gy`, `radiation`, `neutron_energy_mev?` | H_T = w_R · D (Sv); full ICRP 103 neutron continuous function. |
| `rad_effective_half_life` | `t_phys`, `t_bio` | 1/T_eff = 1/T_phys + 1/T_bio. |
| `rad_occupational_limits` | — | ICRP 103 AND US 10 CFR 20 limits side by side. |
| `rad_shielding_thickness` | `energy_kev`, `material`, `transmission` | Required slab thickness; narrow-beam, log-log XCOM interpolation. |
| `rad_alara` | `dose_rate_msv_h_ref`, `distance_ref_m`, `distance_worker_m`, `time_h`, `shielding_transmission?` | Combined time × distance × shielding dose estimate. |

## Example uses

- **Co-60 dose rate at 0.5 m.** 1 GBq Co-60 at 0.5 m →
  `rad_dose_rate { isotope: "Co-60", activity_gbq: 1.0, distance_m: 0.5 }`
  → ~1.4 mSv/h (vs ~0.35 mSv/h at 1 m).
- **Lead shielding for 1.25 MeV photons.** Drop to 1 % transmission →
  `rad_shielding_thickness { energy_kev: 1250, material: "lead",
  transmission: 0.01 }` → narrow-beam thickness in cm. Apply a buildup
  factor for design.
- **ALARA trade.** Halving exposure time and doubling distance gives
  the dose cut explicitly — `rad_alara` reports each axis's
  contribution.

## Notes

- **Narrow-beam attenuation only.** No buildup factor is applied.
  For shielding **design** multiply by an appropriate B factor
  (~1.5–4× for thick Pb at MeV energies). For reach-and-grab work the
  narrow-beam number is the right ballpark.
- **Idealized point sources.** `rad_dose_rate` assumes a bare, isotropic
  point source. Real sources have geometry, encapsulation, and
  self-absorption.
- **US vs international limits** disagree on the lens-of-eye limit and
  on declared-pregnancy embryo limits. `rad_occupational_limits` returns
  both so the model can pick the right regime.

## See also

- [tools.md](../tools.md)
- [skills/nuclear.md](nuclear.md) — decay chains, Q-values, atomic
  masses.
- [skills/atmospheric.md](atmospheric.md) — `atm_space_weather_kp` for
  cosmic-ray ionization context.
