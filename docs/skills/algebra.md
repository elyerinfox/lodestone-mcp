# Algebra — `algebra_solve`, `algebra_formula`, `algebra_formula_list`

|  |  |
| --- | --- |
| **Module** | [`src/skills/algebra.rs`](../../src/skills/algebra.rs) |
| **Tools** | `algebra_solve`, `algebra_formula`, `algebra_formula_list` |
| **Network** | none (local) |
| **Default** | on; gateable via `[tools]` |

## What it does
Solves single-variable (in `x`) linear/quadratic equations (was `math_solve`), and
computes named algebra/combinatorics formulas via the shared formula registry.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `algebra_solve` | `equation` | Solve `2x + 3 = 7` or `x^2 - 5x + 6 = 0`; reports root(s), complex roots, or no solution. |
| `algebra_formula` | `name`, `args` | Compute a named formula (`combinations`, `permutations`, `factorial`, `quadratic_discriminant`). |
| `algebra_formula_list` | `filter?` | Discover the algebra/combinatorics formula ids. |

## Example uses
- `algebra_solve { equation: "x^2 - 5x + 6 = 0" }` → roots 2 and 3.
- `algebra_formula { name: "combinations", args: { n: 52, r: 5 } }` → 2598960.

## See also
[tools.md](../tools.md) · [arithmetic.md](arithmetic.md)
