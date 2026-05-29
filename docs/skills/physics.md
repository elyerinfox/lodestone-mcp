# Physics — `physics_formula`, `physics_formula_list`, `physical_constant`, `wave_frequency`

|  |  |
| --- | --- |
| **Module** | [`src/skills/physics.rs`](../../src/skills/physics.rs) |
| **Tools** | `physics_formula`, `physics_formula_list`, `physical_constant`, `wave_frequency` |
| **Network** | none (local) |
| **Default** | on; gateable via `[tools]` |

## What it does
~70 named physics formulas computed via the shared registry, a table of SI physical
constants, and a wave frequency/wavelength/period converter. SI units throughout;
**angles in degrees**; optional inputs (gravity `g`, emissivity `eps`, an angle
`theta`) default sensibly.

Categories: **mechanics, gravitation, electromagnetism, thermodynamics, waves,
optics, relativity, atomic, nuclear, fluids**. "Solve for a different variable"
variants are separate ids (e.g. `ohms_law_voltage` / `_current` / `_resistance`).

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `physics_formula` | `name`, `args` | Compute, e.g. `kinetic_energy`, `mass_energy`, `ideal_gas_pressure`, `coulombs_law`. |
| `physics_formula_list` | `filter?` | Discover formula ids; `filter` matches a category or id/equation substring. |
| `physical_constant` | `name?` | SI constants (c, G, h, k_B, R, e, N_A, σ, …); `name` filters, omit for all. |
| `wave_frequency` | `frequency_hz?`, `wavelength_m?`, `speed_m_s?` | Convert frequency ↔ wavelength ↔ period (v = f·λ). |

## Example uses
- `physics_formula { name: "kinetic_energy", args: { m: 2, v: 3 } }` → `KE = 9 J`.
- `physics_formula { name: "ideal_gas_pressure", args: { n: 1, T: 300, V: 0.0224 } }`.
- `physics_formula_list { filter: "relativity" }` → the relativity ids.
- `physical_constant { name: "planck" }`.

## See also
[tools.md](../tools.md) · [formula engine](../../src/skills/formula.rs)
