# ODE integration — `ode_rk4`

|  |  |
| --- | --- |
| **Module** | [`src/skills/ode.rs`](../../src/skills/ode.rs) |
| **Tools** | `ode_rk4` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `meval` (expression parser/evaluator) |

## What it does

Classical fourth-order Runge-Kutta integrator for a system of first-order
ODEs `dy/dt = f(t, y)`. Each component of `f` is a `meval` expression
referring to `t` and `y0`, `y1`, … — so the caller doesn't have to compile
any code to integrate a custom system.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `ode_rk4` | `rhs`, `y0`, `t_start`, `t_end`, `steps?` | Integrate the system; returns the `(t, y)` trajectory. |

`rhs` is a vector of strings, one per state variable. `y0` is the initial
state of the same length. `steps` defaults to 100, max 10 000.

## Example uses

- **Exponential decay** — `rhs: ["-y0"], y0: [1.0], t_start: 0, t_end: 5` —
  recovers e⁻ᵗ to 1e-6 at default step count.
- **Damped harmonic oscillator** — two-state form `y0 = x`, `y1 = v`,
  `rhs: ["y1", "-k*y0 - c*y1"]` after substituting numeric `k`, `c`.
- **2-D projectile** — paired with [`traj_projectile_drag`](trajectory.md)
  when wind / drag is also being modeled; pure-gravity prototypes can use
  `rhs: ["y2", "y3", "0", "-9.81"]` for `(x, y, vx, vy)`.

## Notes

- **Fixed-step**. For stiff systems or large rate variations, increase
  `steps`. Adaptive integrators are out of scope for v1.
- **Expression scope**. Only `t` and `y0..yN` are available — no SymPy-style
  named parameters. Substitute numeric constants into the expression before
  the call.

## See also

- [tools.md](../tools.md)
- [skills/linalg.md](linalg.md) — linear ODEs often start from a state-space
  formulation.
- [skills/trajectory.md](trajectory.md) — ballistics simulator uses RK4
  internally.
