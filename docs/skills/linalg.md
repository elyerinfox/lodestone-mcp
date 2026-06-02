# Linear algebra — `linalg_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/linalg.rs`](../../src/skills/linalg.rs) |
| **Tools** | `linalg_solve`, `linalg_lstsq`, `linalg_svd`, `linalg_eigen`, `linalg_qr`, `linalg_inv`, `linalg_det`, `linalg_rank`, `linalg_norm`, `linalg_matmul` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `nalgebra` (dynamic `DMatrix<f64>` / `DVector<f64>`) |

## What it does

Linear-algebra primitives over dense real matrices. Inputs are row-major
JSON arrays (`[[1,2],[3,4]]`); outputs are the same shape (or a vector
where the math demands it). Pure-Rust, no BLAS/LAPACK.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `linalg_solve` | `a`, `b` | Solve A·x = b via LU. |
| `linalg_lstsq` | `a`, `b` | Least-squares A·x ≈ b (via SVD); returns x + residual norm. |
| `linalg_svd` | `matrix` | SVD: U, Σ (singular values), Vᵀ. |
| `linalg_eigen` | `matrix` | Real / complex eigenvalues + eigenvectors. |
| `linalg_qr` | `matrix` | QR decomposition. |
| `linalg_inv` | `matrix` | Matrix inverse (errors if singular). |
| `linalg_det` | `matrix` | Determinant. |
| `linalg_rank` | `matrix` | Numerical rank via SVD with tolerance. |
| `linalg_norm` | `vector?` or `matrix?`, `kind?` | `l1`, `l2` (default), `inf`, or `fro` (matrix only). |
| `linalg_matmul` | `a`, `b` | Matrix multiply A·B. |

## Example uses

- **Polynomial fit.** Build the Vandermonde `a`, feed observed `b`, call
  `linalg_lstsq` — get the polynomial coefficients + residual.
- **PCA-style decomposition.** `linalg_svd` on a feature matrix → principal
  components in `vt` rows, importance in `singular_values`.
- **Stability check.** `linalg_eigen` to read off the spectrum of a state
  transition matrix when wiring up a Kalman filter (see
  [tracking.md](tracking.md)).

## Notes

- Inputs must be rectangular (ragged rows are rejected with a clear error).
- `linalg_inv` errors on near-singular matrices; for rank-deficient systems
  prefer `linalg_lstsq` (it uses SVD internally and is robust).

## See also

- [tools.md](../tools.md)
- [skills/ode.md](ode.md) — ODE integrator that often pairs with linear
  state-space models.
- [skills/tracking.md](tracking.md) — Kalman filter consumes the same
  matrix shapes.
