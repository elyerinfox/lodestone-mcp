# Machinist / mechanical engineering — `mach_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/machinist.rs`](../../src/skills/machinist.rs) |
| **Tools** | `mach_cutting_speed`, `mach_feed_rate`, `mach_mrr_milling`, `mach_cutting_power`, `mach_surface_finish_turning`, `mach_beam_deflection`, `mach_section_inertia`, `mach_stress_strain`, `mach_bolt_torque`, `mach_thread_spec`, `mach_material`, `mach_hardness_convert` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

A working machinist + mechanical-engineer toolkit: cutting kinematics
(RPM, feed, MRR), Kienzle cutting power, theoretical surface finish in
turning, beam deflection (Shigley table A-9), area moment of inertia,
axial stress/strain, bolt-torque from preload via Shigley's K-factor
table, and lookup against vendored UNC + ISO metric coarse thread
tables, material-property table, and ASTM E140 hardness conversion.

## Source citations

- **Cutting kinematics / threads / tap-drills**: *Machinery's Handbook*
  31st ed. (Industrial Press, 2020), §§ "Cutting Speeds and Feeds",
  "Screw Threads".
- **Specific cutting energy (Kienzle)**: Sandvik Coromant,
  *Specific Cutting Force k_c*
  (<https://www.sandvik.coromant.com/en-us/knowledge/materials/specific-cutting-force>).
  k_c = k_c1 · h_m^(−m_c).
- **Surface finish**: ISO 4287:1997, *Geometrical Product
  Specifications — Surface Texture*.
- **Beam deflection / inertias**: R. G. Budynas & J. K. Nisbett,
  *Shigley's Mechanical Engineering Design*, 11th ed., McGraw-Hill 2020,
  Tables A-9 and A-18.
- **Bolt torque** (T = K·d·F): Shigley's Eqs. 8-27, 8-31, 8-32. Nut
  factors from Shigley Table 8-15.
- **Material properties**: MatWeb data sheets and ASM Handbook, Vol 1-2,
  typical certified values for each named alloy.
- **Threads**: ASME B1.1 (UNC/UNF), ISO 261 (metric coarse). 75 %
  engagement tap drills.
- **Hardness conversion**: ASTM E140-12b Table 1 (non-austenitic steels).

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `mach_cutting_speed` | `v`, `diameter`, `units?` | RPM from cutting speed (metric or imperial). |
| `mach_feed_rate` | `feed_per_tooth`, `teeth`, `rpm` | F = f_z · z · N. |
| `mach_mrr_milling` | `a_e_mm`, `a_p_mm`, `feed_mm_min` | MRR in cm³/min. |
| `mach_cutting_power` | `mrr_cm3_min`, `h_m_mm`, `material` | Kienzle P (kW); vendored k_c1/m_c for 5 materials. |
| `mach_surface_finish_turning` | `feed_mm_rev`, `nose_radius_mm` | Theoretical Ra / Rt (μm). |
| `mach_beam_deflection` | `case`, `load`, `length_m`, `e_pa`, `i_m4` | Max deflection for four common beam cases. |
| `mach_section_inertia` | `shape`, `b_m?`, `h_m?`, `d_m?` | I for rectangle (b·h³/12) or round (π·d⁴/64). |
| `mach_stress_strain` | `force_n`, `area_m2`, `e_pa?` | σ = F/A, ε = σ/E if E supplied. |
| `mach_bolt_torque` | `diameter_m`, `preload_n`, `condition` | T = K·d·F; nut factor by condition. |
| `mach_thread_spec` | `thread` | UNC or ISO metric thread + tap-drill. |
| `mach_material` | `material` | Yield, ultimate, ρ, E, ν for common alloys. |
| `mach_hardness_convert` | `hrc` | HRC → HV → HB (ASTM E140). |

## Example uses

- **Set spindle.** Milling 6061 at V = 100 m/min on a 20 mm endmill →
  `mach_cutting_speed { v: 100, diameter: 20 }` → ~1592 rpm. Pair with
  `mach_feed_rate` to drive your post.
- **Cutting power sanity.** 10 cm³/min in steel 1020 at 0.05 mm chip
  thickness → ~4 kW.
- **Bolt-up.** M12 8.8 lubricated to preload ≈ 30 kN →
  `mach_bolt_torque { diameter_m: 0.012, preload_n: 30000,
  condition: "lubricated" }` → ~65 N·m.
- **Tap drill.** `mach_thread_spec { thread: "M8" }` → 6.80 mm drill.

## Notes

- The vendored material and k_c1 values are **typical**. Always verify
  against the actual material certificate for design work.
- Shigley's nut-factor table puts dry-as-received K at 0.30 (not 0.20 —
  the popular shorthand). The tool surfaces this difference explicitly
  to avoid surprises.
- Surface-finish formula is **theoretical**. Real Ra is typically
  20-50 % higher due to vibration, built-up edge, and runout.

## See also

- [tools.md](../tools.md)
- [skills/cnc.md](cnc.md) — emit G-code that uses these speeds & feeds.
- [skills/physics.md](physics.md) — `physical_constant` for fundamental
  constants.
