# Trigonometry — `trig_formula`, `trig_formula_list`

|  |  |
| --- | --- |
| **Module** | [`src/skills/trigonometry.rs`](../../src/skills/trigonometry.rs) |
| **Tools** | `trig_formula`, `trig_formula_list` |
| **Network** | none (local) |
| **Default** | on; gateable via `[tools]` |

## What it does
Named trigonometry formulas via the shared registry: `sin`/`cos`/`tan` and their
inverses (**in degrees**), degree↔radian conversion, the law of sines and law of
cosines (angle), right-triangle relations, arc length, and sector area.
(`arithmetic_eval` also evaluates `sin`/`cos`/`tan` in radians.)

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `trig_formula` | `name`, `args` | Compute, e.g. `sine`, `arccosine`, `law_of_cosines_angle`, `arc_length`. |
| `trig_formula_list` | `filter?` | Discover the trigonometry formula ids. |

## Example uses
- `trig_formula { name: "sine", args: { theta: 30 } }` → 0.5.
- `trig_formula { name: "law_of_cosines_angle", args: { a: 3, b: 4, c: 5 } }` → 90°.

## See also
[tools.md](../tools.md) · [geometry.md](geometry.md)
