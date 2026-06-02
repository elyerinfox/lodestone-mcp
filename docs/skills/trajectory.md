# Trajectory mechanics — `traj_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/trajectory.rs`](../../src/skills/trajectory.rs) |
| **Tools** | `traj_projectile_drag`, `traj_hohmann`, `traj_reentry_heating` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

Three classical trajectory primitives — a drag + wind projectile RK4
integrator, the Hohmann two-impulse transfer, and Sutton-Graves
stagnation-point reentry heating. Pairs naturally with
[`atmospheric`](atmospheric.md) (air density) and
[`ode`](ode.md) (for custom dynamics).

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `traj_projectile_drag` | `v0_m_s`, `angle_deg`, `mass_kg`, `cd`, `area_m2`, `rho_kg_m3?`, `dt_s?`, `t_max_s?`, `wind_m_s?` | RK4 integration with drag `F = ½·ρ·Cd·A·v|v|` + optional headwind. Returns trajectory + range / max-altitude / time-of-flight. |
| `traj_hohmann` | `mu`, `r1_m`, `r2_m` | Hohmann transfer: Δv₁, Δv₂, total Δv, transfer time. |
| `traj_reentry_heating` | `velocity_m_s`, `density_kg_m3`, `nose_radius_m` | Sutton-Graves stagnation-point heat flux: q = 1.74e-4 · √(ρ/Rₙ) · v³. |

## Example uses

- **Artillery / sports ballistics.** "How far does a 1 kg sphere
  (Cd≈0.47, A=0.01 m²) go at 100 m/s @ 45°?" → `traj_projectile_drag`
  → range, apogee, time of flight.
- **Orbit raising.** LEO → GEO Δv budget: `traj_hohmann { mu: 3.986e14,
  r1_m: 7.0e6, r2_m: 4.2164e7 }`.
- **Reentry envelope.** Sweep `velocity_m_s` through
  `traj_reentry_heating` at the relevant `density_kg_m3` from
  `atm_isa` — the q-vs-v envelope sizing your TPS.

## Notes

- The projectile model is 2-D in the launch plane (x-down-range, y-up).
  Side wind / cross-range work isn't included.
- Hohmann is the two-impulse, coplanar, ideal-impulse transfer — useful
  ballpark, not a flight-plan substitute.

## See also

- [tools.md](../tools.md)
- [skills/ode.md](ode.md) — custom dynamics via `ode_rk4`.
- [skills/atmospheric.md](atmospheric.md) — `atm_isa` density / pressure.
- [skills/satellite.md](satellite.md) — SGP4 propagation for real orbits.
