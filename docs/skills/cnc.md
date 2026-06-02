# CNC / OpenSCAD — `gcode_*` and `scad_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/cnc.rs`](../../src/skills/cnc.rs) |
| **Tools** | `gcode_drill_hole`, `gcode_bolt_pattern`, `gcode_parse_summary`, `scad_box`, `scad_cylinder`, `scad_sphere`, `scad_flange` |
| **Network** | none — local source generation / parsing |
| **Default** | on; gateable via `[tools]` |

## What it does

A small toolkit for emitting portable G-code (motion + drilling +
bolt-circle patterns) and OpenSCAD source for common geometry. Also a
G-code parser that summarizes command counts, axis travel, bounding box,
and modal state so generated programs can be sanity-checked before
sending them to the controller.

## Source citations

- **G-code dialect**: NIST RS-274/NGC v3 — Kramer, Proctor, Messina,
  *The NIST RS274NGC Interpreter — Version 3*, NIST Tech. Note (2000).
  This is the de-facto reference (and what LinuxCNC implements).
  Targeting it gives maximal portability to Grbl (subset) and Marlin
  (with known limitations called out in the tool descriptions).
- **ISO 6983-1 (1982)** is the nominal international G-code standard
  but is rarely implemented literally — we treat RS-274/NGC as the
  canonical reference dialect.
- **OpenSCAD language reference**:
  <https://en.wikibooks.org/wiki/OpenSCAD_User_Manual>.

## Tools

### G-code

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `gcode_drill_hole` | `x`, `y`, `depth`, `safe_z`, `plunge_feed`, `rpm`, `units?`, `tool?` | Single hole at (X, Y); RS-274/NGC preamble + tool change + plunge. |
| `gcode_bolt_pattern` | `pcd`, `n`, `depth`, `safe_z`, `plunge_feed`, `rpm`, `cx?`, `cy?`, `start_angle_deg?`, `units?`, `tool?` | N evenly spaced drilled holes on a pitch-circle diameter. |
| `gcode_parse_summary` | `gcode` | Per-command counts (G0/G1/.../M*), bbox, axis travel, modal state. |

### OpenSCAD

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `scad_box` | `x`, `y`, `z`, `center?` | `cube([x,y,z], center=…)`. |
| `scad_cylinder` | `height`, `r?` / `r1?+r2?`, `fn_?`, `center?` | `cylinder(h=…, r/r1+r2=…, $fn=…, center=…)`. |
| `scad_sphere` | `r`, `fn_?` | `sphere(r=…, $fn=…)`. |
| `scad_flange` | `od`, `thickness`, `pcd`, `n`, `hole_r`, `bore?`, `fn_?` | Idiomatic `difference()` flange with N clearance holes on a PCD and optional central bore. |

## Example uses

- **Drill a 5 mm-deep hole at (10, 20)** with 100 mm/min plunge at
  2000 RPM → `gcode_drill_hole { x: 10, y: 20, depth: 5, safe_z: 5,
  plunge_feed: 100, rpm: 2000 }`.
- **6-hole bolt pattern on 80 mm PCD** centred at the origin:
  `gcode_bolt_pattern { pcd: 80, n: 6, ... }`.
- **Sanity check generated G-code** before sending to Grbl —
  `gcode_parse_summary` reports the bbox so collisions in Z are obvious
  in advance.
- **A flange with 8 M6 clearance holes** on a 120 mm PCD →
  `scad_flange { od: 160, thickness: 10, pcd: 120, n: 8, hole_r: 3.5 }`.

## Notes

- **Z+ is up.** Standard for CNC + LinuxCNC + Grbl + Marlin (3-axis).
- **Marlin caveats** (drilling-only tools dodge most of these): Marlin
  G2/G3 supports only I/J (not R), only G17; M30 may behave like M2;
  M6 needs tool-change support enabled.
- **Grbl caveats**: no canned cycles (G81/G82/G83) — we emit explicit
  G0/G1 sequences, which work everywhere.
- **The emitted preamble is `G17 G21 G90 G94`** (XY plane, mm,
  absolute, feed/min) — swap to `G20` for inches via `units:
  "imperial"`. Switch on this and the entire program inherits the unit.
- **OpenSCAD output is plain source** — the tool doesn't render. Pipe
  the result into `openscad -o out.stl model.scad` if you need the
  mesh.

## See also

- [tools.md](../tools.md)
- [skills/machinist.md](machinist.md) — feeds & speeds + thread specs
  + material properties feed straight into these G-code emitters.
