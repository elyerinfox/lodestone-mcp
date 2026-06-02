# Optimization & operations research — `opt_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/optimization.rs`](../../src/skills/optimization.rs) |
| **Tools** | `opt_tsp_2opt`, `opt_shortest_path` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `pathfinding` |

## What it does

Two classical combinatorial-optimization helpers — a TSP solver
(nearest-neighbour seed refined by 2-opt) and Dijkstra shortest path on
a directed weighted graph. The TSP solver is a heuristic — fine for the
~30-node range a model would actually use.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `opt_tsp_2opt` | `distances` (symmetric square matrix, zero diagonal) | Tour visiting every node + total tour length. |
| `opt_shortest_path` | `edges` (list of `[from, to, weight]`), `start`, `goal` | Min-cost path (vector of nodes) + total cost. |

## Example uses

- **Multi-stop delivery.** Build a pairwise-distance matrix from
  `geo_vincenty_inverse` over a city's drop-points →
  `opt_tsp_2opt` returns a route the model can name in tool form.
- **Routing.** Feed `edges` straight to `opt_shortest_path` for a
  network-cost problem (latency-weighted graph of nodes, dependency
  resolution).

## Notes

- `opt_tsp_2opt` is **heuristic** — for very small (≤ 15 nodes) a true
  optimum is computable, but the model doesn't need it for the cases
  this surface targets.
- LP / MILP / max-flow / SA / GA are deliberately out of scope for
  v0.1.2 to avoid dragging in a numerical-programming dep stack.

## See also

- [tools.md](../tools.md)
- [skills/geodesy.md](geodesy.md) — distances to populate `distances`.
- [skills/tracking.md](tracking.md) — Hungarian assignment for a related
  matching problem.
