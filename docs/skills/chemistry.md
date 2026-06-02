# Chemistry — `chem_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/chemistry.rs`](../../src/skills/chemistry.rs) |
| **Tools** | `chem_periodic_table`, `chem_molar_mass`, `chem_formula_hill`, `chem_balance_equation`, `chem_ph`, `chem_buffer`, `chem_ideal_gas`, `chem_dilution`, `chem_gibbs`, `chem_radioactive_decay` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `num-bigint`, `num-integer`, `num-traits` (exact integer arithmetic for the equation balancer) |

## What it does

A working chemistry tool-set: periodic-table lookup, molar mass from a
formula string (parentheses + hydrates), exact integer equation balancing,
acid/base/buffer pH, the ideal gas law, M₁V₁=M₂V₂ dilution, ΔG = ΔH − TΔS,
and first-order radioactive decay.

## Source citations

- **Atomic weights** — IUPAC CIAAW, *Standard Atomic Weights 2021*
  (Prohaska et al., *Pure Appl. Chem.* 2022, 94(5):573-600). Abridged
  values; conventional single value used for elements with intervals.
- **Group 3 placement** of Lu and Lr — IUPAC Provisional Report (Scerri
  et al., 2021).
- **Gas constant** R = 8.314 462 618 J/(mol·K) — exact value defined by
  the 2019 SI redefinition (R = N_A · k_B).
- **Henderson-Hasselbalch** — Henderson (1908), Hasselbalch (1917).
- **Hill ordering** — Hill, *J. Am. Chem. Soc.* 1900, 22(8):478-494.
- **Equation balancer** — fraction-free Gauss-Jordan on the integer
  element-coefficient matrix (cf. Bareiss, *Math. Comp.* 1968), then
  LCM/GCD rationalization. Exact, no floating point.
- **Radioactive decay** — N(t) = N₀ · (½)^(t/t½), λ = ln 2 / t½.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `chem_periodic_table` | `element` (symbol / name / Z) | Atomic number, symbol, name, atomic mass, group, period, common oxidation states, category. |
| `chem_molar_mass` | `formula` | Molar mass (g/mol) for a formula, including parentheses (`Ca(OH)2`) and hydrates (`CuSO4·5H2O` / `CuSO4.5H2O`). |
| `chem_formula_hill` | `formula` | Normalize to Hill order (C, H, then alphabetical). |
| `chem_balance_equation` | `equation` (LHS `=` or `->` RHS) | Smallest positive integer stoichiometry via exact rational arithmetic. Detects infeasibility (element on only one side) and under-determination (nullity > 1). |
| `chem_ph` | `kind`, `concentration_m`, `pka_or_pkb?` | pH / pOH / [H⁺] / [OH⁻] at 25 °C. `kind` ∈ {`strong_acid`, `strong_base`, `weak_acid`, `weak_base`}. Weak species use small-x approximation [H⁺] ≈ √(Ka·C₀). |
| `chem_buffer` | `pka`, `acid_m`, `base_m` | Henderson-Hasselbalch buffer pH. |
| `chem_ideal_gas` | three of `pressure_pa` / `volume_m3` / `moles` / `temperature_k` | PV = nRT; the missing one is returned. |
| `chem_dilution` | three of `c1_m` / `v1_l` / `c2_m` / `v2_l` | M₁V₁ = M₂V₂. |
| `chem_gibbs` | `delta_h_kj`, `delta_s_j_per_k`, `temperature_k` | ΔG = ΔH − T·ΔS; reports spontaneity sign. |
| `chem_radioactive_decay` | `n0`, `half_life_s`, `time_s` | First-order decay; returns N(t), fraction remaining, and λ. |

## Example uses

- **Lookup.** `chem_periodic_table { element: "Fe" }` → atomic mass
  55.845, group 8, period 4, common ox states {2, 3}.
- **Molar mass of glucose.** `chem_molar_mass { formula: "C6H12O6" }`
  → 180.16 g/mol.
- **Balance propane combustion.**
  `chem_balance_equation { equation: "C3H8 + O2 = CO2 + H2O" }` →
  `C3H8 + 5 O2 = 3 CO2 + 4 H2O`.
- **Buffer for blood-pH calculation** at the carbonic-bicarbonate pKa
  6.10: `chem_buffer { pka: 6.10, base_m: 0.024, acid_m: 0.0012 }`
  → pH ≈ 7.40.
- **Half-life check.** Tc-99m at 6 h half-life over 24 h:
  `chem_radioactive_decay { n0: 1.0, half_life_s: 21600, time_s: 86400 }`
  → 1/16 remaining.

## Notes

- The balancer is **mass-balance only** — redox half-reactions need
  charge balance and aren't covered. Use the chem skill for the species
  inventory then pencil-and-paper the redox half.
- The periodic table omits one-line "use notes" deliberately — the goal
  is a fact lookup, not a chemistry textbook.
- pH calculation assumes 25 °C (Kw = 10⁻¹⁴). Temperature corrections
  for Kw are out of scope.

## See also

- [tools.md](../tools.md)
- [skills/biology.md](biology.md) — protein MW uses the chemistry-style
  monoisotopic mass convention.
- [skills/physics.md](physics.md) — `physical_constant` provides R, k_B,
  N_A, h, c, …
