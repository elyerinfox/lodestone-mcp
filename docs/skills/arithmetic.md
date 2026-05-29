# Arithmetic — `arithmetic_eval`

|  |  |
| --- | --- |
| **Module** | [`src/skills/arithmetic.rs`](../../src/skills/arithmetic.rs) |
| **Tools** | `arithmetic_eval` |
| **Network** | none (local) |
| **Default** | on; gateable via `[tools]` |

## What it does
Evaluates a free-form arithmetic/scientific expression via the `meval` evaluator:
functions (`sqrt`, `sin`, `cos`, `tan`, `ln`, `log`, `exp`, `abs`, `floor`, `ceil`,
…), constants (`pi`, `e`), and `^` for powers. Implicit multiplication like `2x` or
`2(3+4)` is normalized. (Was `math_eval`.)

For *named* formulas use the field `*_formula` tools (algebra/geometry/trigonometry/
physics); for equations use `algebra_solve`.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `arithmetic_eval` | `expression` | Evaluate, e.g. `pi*5^2`, `sqrt(2)`, `sin(pi/2)`. |

## See also
[tools.md](../tools.md) · [algebra.md](algebra.md)
