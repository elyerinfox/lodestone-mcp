# color — hex/RGB/HSL/Lab convert, WCAG contrast, perceptual blend

|  |  |
| --- | --- |
| **Module** | [`src/skills/color.rs`](../../src/skills/color.rs) |
| **Tools** | `color_convert`, `color_contrast`, `color_blend` |
| **Network** | none — pure local compute |
| **Default** | on (no config gate) |

## What it does

Three tools covering color math LLMs reliably mess up:

- **`color_convert { color }`** — parse one color (hex `#aabbcc` or
  `#abc`, `rgb()/rgba()`, `hsl()/hsla()`) and emit every supported form
  at once: hex, RGB 0–255, HSL deg/%, CIE L\*a\*b\* under D65, linear
  sRGB.
- **`color_contrast { foreground, background }`** — WCAG 2.1 SC 1.4.3
  contrast ratio + pass/fail at AA / AAA for normal and large text.
  Output 1.0 (no contrast) → 21.0 (black on white).
- **`color_blend { a, b, t?, space? }`** — blend two colors at factor
  `t` (default 0.5). `space="linear"` (default) does the perceptually-
  correct linear-sRGB lerp; `space="srgb"` does the naive sRGB lerp
  that most CSS tools do (visibly wrong but matches editor preview).

## Sources

- sRGB / Rec. 709 luma coefficients (Y' = 0.2126 R + 0.7152 G + 0.0722 B).
- CIE 1976 L\*a\*b\* via the D65 illuminant.
- WCAG 2.1 SC 1.4.3 contrast formula
  ([W3C Recommendation, 2018](https://www.w3.org/TR/WCAG21/#contrast-minimum)).
- HSL ↔ RGB conversion: CSS Color Module Level 3 §4.2.4.

## Example flow

```
1. color_convert { color: "#3498db" }
   → rgb=[52, 152, 219], hsl≈[204°, 70%, 53%], lab=...

2. color_contrast { foreground: "#3498db", background: "#ffffff" }
   → ratio ≈ 3.1; passes AA-large, fails AA-normal

3. color_blend { a: "#3498db", b: "#ffffff", t: 0.3 }
   → perceptual mix; check contrast again to find an AA-passing variant
```

## See also

- [`docs/golden-rules.md`](../golden-rules.md) — golden rule 1 (keyless),
  golden rule 12 (citations).
