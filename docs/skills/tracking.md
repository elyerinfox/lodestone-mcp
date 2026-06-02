# Estimation & tracking — `track_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/tracking.rs`](../../src/skills/tracking.rs) |
| **Tools** | `track_kalman_step`, `track_hungarian`, `track_ransac_line` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `nalgebra`, `pathfinding`, `rand` |

## What it does

Building-block estimators a model can chain into a tracker — a single
predict + update Kalman step, optimal assignment for measurement-to-track
association, and a 2-D RANSAC line fit. The state is **not** carried
across calls — every step takes the previous state + covariance as
arguments and returns the new ones. That's deliberate: state belongs in
the conversation, not in the server.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `track_kalman_step` | `x` (n), `p` (n×n), `f` (n×n), `q` (n×n), `h` (m×n), `r` (m×m), `z` (m) | One predict + update; returns posterior `x` + `p`, `innovation`, and **NIS** (normalized innovation squared) for chi-squared gating. |
| `track_hungarian` | `cost` (workers × jobs) | Kuhn-Munkres optimal assignment minimizing total cost; returns `assignment[i]` = job index or −1, plus the achieved total cost. |
| `track_ransac_line` | `points`, `threshold`, `iterations?` | RANSAC fit of `ax + by + c = 0` (normalized); returns coefficients, inlier indices, and inlier count. |

## Example uses

- **Constant-velocity tracker.** `f = [[1,Δt],[0,1]]`, `h = [[1,0]]`,
  feed measurements one at a time through `track_kalman_step`. NIS
  outside the 95 % chi-squared bound flags a missed maneuver.
- **Data association.** Build a cost matrix of `||predicted − measured||`
  pairs → `track_hungarian` → use the assignment to update each track.
- **Robust line fit.** `track_ransac_line` on noisy 2-D points (≥ 50 %
  outliers) — the inlier set is the line.

## Notes

- Quaternion / nonlinear EKF / UKF / particle filter would each be their
  own tool — out of scope for v0.1.2.
- `track_hungarian` minimizes; for maximization, negate the cost matrix.
- `track_ransac_line` has no built-in determinism — set `iterations`
  high enough to overcome the outlier ratio.

## See also

- [tools.md](../tools.md)
- [skills/linalg.md](linalg.md) — matrix shapes feed straight in.
- [skills/quaternion.md](quaternion.md) — pair for attitude tracking.
- [skills/radar.md](radar.md) — detections → tracker pipeline.
