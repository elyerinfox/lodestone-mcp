# Math, geo & waves — `math_eval` / `math_solve` / `geo_distance` / `geo_azimuth` / `wave_frequency`

|  |  |
| --- | --- |
| **Module** | [`src/skills/math.rs`](../../src/skills/math.rs) |
| **Tools** | `math_eval`, `math_solve`, `geo_distance`, `geo_azimuth`, `wave_frequency` |
| **Network** | local-only |
| **Default** | on |
| **Config** | none |

## What it does
Numeric computation — all local, no network. `math_eval` evaluates an
arithmetic/scientific expression (via the `meval` evaluator); `math_solve` solves a
single-variable linear or quadratic equation in `x`; `geo_distance` / `geo_azimuth` do
great-circle geometry between two coordinates; `wave_frequency` relates frequency,
wavelength, and period (v = f·λ).

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `math_eval` | `expression` | Evaluate an expression: arithmetic, functions (sqrt, sin, cos, tan, ln, log, exp, abs, floor, ceil, …), constants (pi, e), `^` for powers. Implicit multiplication like `2x` / `2(…)` is normalized. |
| `math_solve` | `equation` | Solve a linear/quadratic equation in `x` (e.g. `2x + 3 = 7`, `x^2 - 5x + 6 = 0`; no `=` means `… = 0`). Reports root(s), double/complex roots, or no/identity solution; higher degrees are rejected. |
| `geo_distance` | `lat1`, `lon1`, `lat2`, `lon2` | Great-circle (haversine) distance between two decimal-degree coordinates, in km and miles. |
| `geo_azimuth` | `lat1`, `lon1`, `lat2`, `lon2` | Initial bearing (forward azimuth, 0=N/90=E) with a 16-point compass label, plus the back azimuth. |
| `wave_frequency` | `frequency_hz?`, `wavelength_m?`, `speed_m_s?` | Convert frequency ↔ wavelength ↔ period. Give exactly one of `frequency_hz` / `wavelength_m`; `speed_m_s` defaults to the speed of light (use ~343 for sound in air). |

## Configuration & gating
No configuration. Each tool is independently gateable via `[tools]`. Coordinates out of
range (lat −90..90, lon −180..180) and non-positive speeds return clear errors.

## Example uses
- **Plan a route leg** — `geo_distance` for the leg length plus `geo_azimuth` for the heading.
- **Solve algebra** — `math_solve` a quadratic, or `math_eval` a geometry formula like `pi*5^2`.
- **Radio/acoustics** — `wave_frequency` to get the wavelength of a 2.4 GHz signal.

## See also
[tools.md](../tools.md)
