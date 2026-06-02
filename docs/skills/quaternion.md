# Quaternion algebra & attitude — `quat_*`, `frame_dcm_from_euler`

|  |  |
| --- | --- |
| **Module** | [`src/skills/quaternion.rs`](../../src/skills/quaternion.rs) |
| **Tools** | `quat_from_euler`, `quat_to_euler`, `quat_compose`, `quat_rotate`, `quat_conjugate`, `quat_normalize`, `quat_slerp`, `frame_dcm_from_euler` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `nalgebra::{Quaternion, UnitQuaternion}` |
| **Convention** | Hamilton, scalar-first (`w, x, y, z`); roll-pitch-yaw about X-Y-Z. Angles in **radians**. |

## What it does

Quaternion math plus an Euler → direction-cosine-matrix helper. Intended for
attitude integration, sensor fusion, and IMU / GNSS pipelines that need to
move between Euler angles, quaternions, and rotation matrices without sign
ambiguity.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `quat_from_euler` | `roll`, `pitch`, `yaw` (rad) | Roll-pitch-yaw → unit quaternion. |
| `quat_to_euler` | `q` (`[w,x,y,z]`) | Quaternion → Euler angles. |
| `quat_compose` | `a`, `b` | Hamilton product a·b (apply `b` then `a`). |
| `quat_rotate` | `q`, `v` (`[x,y,z]`) | Rotate the 3-vector by the quaternion. |
| `quat_conjugate` | `q` | Conjugate (inverse for a unit quaternion). |
| `quat_normalize` | `q` | Renormalize to unit length (numeric drift correction). |
| `quat_slerp` | `a`, `b`, `t` | Spherical linear interpolation, `t ∈ [0,1]`. |
| `frame_dcm_from_euler` | `roll`, `pitch`, `yaw` | 3×3 body→nav direction-cosine matrix. |

## Example uses

- **IMU attitude.** Integrate gyro deltas as small quaternions and
  `quat_compose` them onto the running attitude estimate; periodically
  `quat_normalize`.
- **Animation interpolation.** Two key-frame orientations →
  `quat_slerp` at `t = 0.5` gives the geodesic midpoint (no gimbal lock).
- **Frame transform.** `frame_dcm_from_euler` then `linalg_matmul` against a
  cluster of body-frame positions to express them in the nav frame.

## See also

- [tools.md](../tools.md)
- [skills/linalg.md](linalg.md) — DCMs feed straight into the linalg tools.
- [skills/nav_aiding.md](nav_aiding.md) — IMU drift error budget builds on
  this convention.
