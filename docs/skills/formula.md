# Formula registry — `*_formula` / `*_formula_list`

|  |  |
| --- | --- |
| **Module** | [`src/skills/formula.rs`](../../src/skills/formula.rs) |
| **Tools** | none directly — backs `algebra_formula`, `geometry_formula`, `trig_formula`, `physics_formula` and their `*_list` siblings |
| **Network** | none — local closed-form math |
| **Default** | **on** with each domain that uses it (`[algebra]` / `[geometry]` / `[trig]` / `[physics]`, all default-on) |
| **Config** | none of its own — domains gate via their family flags |

## What it does

A small shared engine the four science / math domains build on top of.
Each domain (`algebra`, `geometry`, `trigonometry`, `physics`) ships a
catalog of **named formulas** that map required inputs → one output via
a pure closure. The registry handles the parts that would otherwise be
copy-pasted four times: input validation, missing-argument errors,
type-safe optional inputs, listing / search, the uniform response
shape.

Each domain's two tools dispatch through here:

| Tool | Behavior |
| --- | --- |
| `<domain>_formula { name, args }` | Look up the formula by id; validate that every required input is supplied; run the closure; return the result with its unit. |
| `<domain>_formula_list { filter? }` | List the catalog: id, inputs (name + unit), optional inputs, output unit, one-line description. With `filter`, substring-match against name + description. |

So `physics_formula { name: "kinetic_energy", args: { m: 5, v: 10 } }`
runs the pure closure `m -> 0.5 * m * v² = 250 J`; `physics_formula_list`
returns the catalog so the model can discover what's available without
guessing.

## Conventions

- **Angles in degrees**, never radians. Domains that take an angle
  declare it as `theta_deg` (or similar) and the closure converts
  internally.
- **SI units throughout**. Mass kg, length m, time s, current A,
  temperature K. Energy J, force N, power W. Frequency Hz.
- **Required inputs are checked before the closure runs**, so closures
  can index `args["x"]` without a `match` ceremony.
- **Optional inputs** are read with the `opt(args, "name")` helper —
  returns `Option<f64>` so the closure can apply a default cleanly.

## Why it lives in its own module

Golden rule 9 (one tool per method) says each named formula deserves
its own explicit dispatch path. But duplicating the catalog +
validation + listing logic across four domains would be a maintenance
trap. The split is: the formulas themselves and their domain naming
live in `algebra.rs` / `geometry.rs` / `trigonometry.rs` /
`physics.rs`; the shared mechanics live here.

A new formula in an existing domain is one entry in that domain's
catalog — no work in this module. A new domain is a new
`<domain>_formula` skill that registers its catalog with this module's
helpers.

## Per-domain catalogs

| Domain | Doc | Examples |
| --- | --- | --- |
| Algebra / combinatorics | [`algebra.md`](algebra.md) | `permutations`, `combinations`, `quadratic_discriminant`, `compound_interest_factor` |
| Geometry | [`geometry.md`](geometry.md) | `circle_area`, `triangle_area_heron`, `sphere_volume`, `regular_polygon_area` |
| Trigonometry | [`trigonometry.md`](trigonometry.md) | `law_of_sines`, `law_of_cosines`, `right_triangle_hypotenuse` |
| Physics | [`physics.md`](physics.md) | `kinetic_energy`, `ohms_law`, `coulombs_law`, `wave_speed` |

## See also

- [`docs/skills/physics.md`](physics.md) — the largest catalog and
  the one with the most cross-cutting unit conventions.
- [`physical_constant` tool](../tools.md#math--science-local-by-field)
  — sits beside the formula tools for things like c, h, e, k.
- [`arithmetic_eval`](arithmetic.md) — for arbitrary expressions that
  aren't a named closed-form.
