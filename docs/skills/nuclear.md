# Nuclear physics — `nuke_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/nuclear.rs`](../../src/skills/nuclear.rs) |
| **Tools** | `nuke_nuclide_lookup`, `nuke_binding_energy`, `nuke_q_value`, `nuke_decay_law`, `nuke_decay_chain`, `nuke_unit_convert` |
| **Network** | none — local compute (vendored tables) |
| **Default** | on; gateable via `[tools]` |

## What it does

Closed-form nuclear physics: semi-empirical mass formula, atomic-mass-unit
↔ MeV conversion, reaction Q-values, decay law and two-step Bateman chain,
plus a vendored subset of nuclide masses + half-lives + decay modes for
common stable and radioactive species.

## Source citations

- **Atomic-mass-unit ↔ MeV**: 1 u = 931.494 103 72 MeV/c² (CODATA 2022 —
  Mohr et al., *Rev. Mod. Phys.* 97, 025002 (2024); NIST CUU).
- **Semi-empirical mass formula coefficients** (Krane defaults):
  K. S. Krane, *Introductory Nuclear Physics*, Wiley 1988, §3.3 /
  Table 3.2: a_V = 15.5, a_S = 16.8, a_C = 0.72, a_A = 23.0, a_P = 34
  MeV; pairing exponent k_P = −3/4.
- **Atomic masses**: AME2020 — Wang, Huang, Kondev, Audi, Naimi,
  *Chinese Phys. C* 45, 030003 (2021); DOI 10.1088/1674-1137/abddaf.
- **Half-lives + decay modes**: NUBASE2020 — Kondev, Wang, Huang, Naimi,
  Audi, *Chinese Phys. C* 45, 030001 (2021).
- **Bateman equation**: Bateman, *Proc. Cambridge Philos. Soc.* 1910,
  15:423-427.
- **Curie**: 1 Ci = 3.7 × 10¹⁰ Bq exactly (definitional).
- **Barn**: 1 b = 10⁻²⁸ m² exactly.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `nuke_nuclide_lookup` | `nuclide` (`U-235`, `Co-60`, `Tc-99m`, `Cs137`, …) | Z, N, A, atomic mass (u), half-life (s), decay modes. |
| `nuke_binding_energy` | `a`, `z` | Bethe-Weizsäcker BE (MeV) + BE/A + term breakdown. |
| `nuke_q_value` | `reactants_u`, `products_u` | Reaction Q (MeV); Q > 0 = exothermic. |
| `nuke_decay_law` | `n0`, `half_life_s`, `time_s` | N(t), A(t) = λ·N, λ, fraction remaining. |
| `nuke_decay_chain` | `n_a0`, `half_life_a_s`, `half_life_b_s`, `time_s` | Bateman two-step A→B; special case for λ_A = λ_B handled. |
| `nuke_unit_convert` | `direction`, `value` | u ↔ MeV/c²; Bq ↔ Ci; barn ↔ cm². |

## Example uses

- **D + T fusion Q-value.** Plug AME2020 atomic masses for D, T, ⁴He,
  and the neutron into `nuke_q_value` → 17.589 MeV.
- **Activity at t.** Co-60 source 1 GBq initially, 5 years later →
  `nuke_decay_law { n0: 1.0e9, half_life_s: 1.6635e8, time_s: 1.578e8 }`
  → ~52 % remaining.
- **Tc-99m generator equilibrium.** Mo-99 → Tc-99m chain — short
  daughter, long parent → transient equilibrium pattern emerges from
  `nuke_decay_chain`.
- **Iron-56 binding.** `nuke_binding_energy { a: 56, z: 26 }` → 487 MeV
  total, 8.79 MeV/nucleon (close to the experimentally observed value;
  Ni-62 is the actual per-nucleon peak at 8.79 MeV — `nuke_nuclide_lookup`
  exposes both).

## Notes

- The SEMF is an approximation; expect a few % discrepancy from
  experimental BE. For precise values use AME2020 atomic masses
  directly via `nuke_nuclide_lookup`.
- The vendored nuclide table covers a curated subset (~30 nuclides). For
  rarer species use external NNDC NuDat 3 / IAEA Live Chart and feed the
  mass into `nuke_q_value` directly.

## See also

- [tools.md](../tools.md)
- [skills/radiology.md](radiology.md) — dose, attenuation, ALARA.
- [skills/chemistry.md](chemistry.md) — first-order
  `chem_radioactive_decay`.
- [skills/physics.md](physics.md) — `physical_constant` provides c, h,
  N_A, k_B, etc.
