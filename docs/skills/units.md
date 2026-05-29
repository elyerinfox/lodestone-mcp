# Unit conversion — `convert_units`

|  |  |
| --- | --- |
| **Module** | [`src/skills/units.rs`](../../src/skills/units.rs) |
| **Tools** | `convert_units` |
| **Network** | local-only |
| **Default** | on |
| **Config** | none |

## What it does
Converts a value between units of the same kind — local, no network. Non-temperature
units convert through a factor to a base unit; temperature is handled as a special
affine case. Cross-kind conversions (e.g. mass → length) are rejected.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `convert_units` | `value`, `from`, `to` | Convert `value` from one unit to another of the same kind. Unit names are case-insensitive aliases. |

Supported kinds and example aliases:
- **length** — mm, cm, m, km, in, ft, yd, mi, nmi
- **mass** — mg, g, kg, t, oz, lb, st
- **volume** — ml, l, m3, tsp, tbsp, cup, pt, qt, gal, floz
- **area** — mm2, cm2, m2, km2, ha, acre, sqft, sqin, sqmi
- **speed** — m/s, km/h, mph, knot, ft/s
- **time** — ns, us, ms, s, min, h, day, week
- **data** — bit, byte, kb/mb/gb/tb (decimal), kib/mib/gib/tib (binary)
- **temperature** — celsius, fahrenheit, kelvin (affine, not factor-based)

## Configuration & gating
No configuration. Unknown units, and conversions between incompatible kinds (including
temperature vs. non-temperature), return clear errors. Gateable via `[tools]`.

## Example uses
- **Imperial ↔ metric** — `convert_units` 26.2 `mi` to `km`, or 5 `lb` to `kg`.
- **Temperature** — `convert_units` 100 `celsius` to `fahrenheit`.
- **Storage sizing** — `convert_units` 1 `gib` to `byte` to reconcile decimal vs. binary sizes.

## See also
[tools.md](../tools.md)
