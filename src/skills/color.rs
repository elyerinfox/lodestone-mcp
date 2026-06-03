//! Color skills (local compute): hex / RGB / HSL / Lab round-trip
//! conversion, WCAG 2.1 contrast ratios, and linear blending. Pure-Rust,
//! no external crate dependency. LLMs hallucinate hex codes for named
//! colors and get HSL → RGB conversion wrong by a few units; these tools
//! give the model deterministic answers.
//!
//! ## Sources
//!
//! - sRGB / Rec. 709 luma coefficients (Y' = 0.2126 R + 0.7152 G + 0.0722 B).
//! - CIE 1976 L*a*b* via the D65 illuminant.
//! - WCAG 2.1 SC 1.4.3 contrast formula (Web Content Accessibility
//!   Guidelines 2.1, W3C Recommendation, 2018).
//! - HSL ↔ RGB conversion: CSS Color Module Level 3, §4.2.4.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// color_convert
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConvertArgs {
    /// Color in any supported form. Accepts:
    /// `#rgb`, `#rrggbb` (with or without `#`), `rgb(r,g,b)`, `rgba(r,g,b,a)`,
    /// `hsl(h, s%, l%)`. Components are clipped to valid ranges.
    color: String,
}

pub struct ColorConvert;
impl Skill for ColorConvert {
    fn name(&self) -> &'static str {
        "color_convert"
    }
    fn description(&self) -> &'static str {
        "Parse one color and emit it in every supported form at once: `hex` (#rrggbb), `rgb` \
         (0-255), `hsl` (deg / %), `lab` (CIE L*a*b* under D65), `linear_rgb` (sRGB inverse \
         gamma applied). Accepts hex (`#aabbcc`, `aabbcc`, `#abc`), `rgb()` / `rgba()`, or \
         `hsl()` input. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConvertArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ConvertArgs>()?;
            let (r, g, b, a) = parse_color(&args.color)?;
            let (h, sl_s, sl_l) = rgb_to_hsl(r, g, b);
            let (lab_l, lab_a, lab_b) = rgb_to_lab(r, g, b);
            let (lr, lg, lb) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
            Ok(text_result(
                json!({
                    "input": args.color,
                    "hex": format!("#{:02x}{:02x}{:02x}", to_u8(r), to_u8(g), to_u8(b)),
                    "rgb": [to_u8(r), to_u8(g), to_u8(b)],
                    "alpha": a,
                    "hsl": {
                        "h_deg": round1(h),
                        "s_pct": round1(sl_s * 100.0),
                        "l_pct": round1(sl_l * 100.0),
                    },
                    "lab": {
                        "L": round2(lab_l),
                        "a": round2(lab_a),
                        "b": round2(lab_b),
                    },
                    "linear_rgb": [round3(lr), round3(lg), round3(lb)],
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Hex with hash",
                args: r##"{"color": "#3498db"}"##,
                note: Some(
                    "Returns rgb=[52, 152, 219], hsl≈[204°, 70%, 53%], plus Lab and linear sRGB.",
                ),
            },
            SkillExample {
                title: "Short-form hex",
                args: r##"{"color": "#abc"}"##,
                note: Some("Expanded to #aabbcc."),
            },
            SkillExample {
                title: "From rgb()",
                args: r#"{"color": "rgb(255, 99, 71)"}"#,
                note: Some("Tomato."),
            },
            SkillExample {
                title: "From hsl()",
                args: r#"{"color": "hsl(120, 100%, 50%)"}"#,
                note: Some("Pure green."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Get the deterministic hex / RGB / HSL / Lab form of a color without guessing.",
            "Convert between two color models when planning a palette.",
            "Spot-check what RGB triplet an HSL value really resolves to.",
        ]
    }
}

// ---------------------------------------------------------------------------
// color_contrast
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ContrastArgs {
    /// Foreground color in any form `color_convert` accepts.
    foreground: String,
    /// Background color in any form `color_convert` accepts.
    background: String,
}

pub struct ColorContrast;
impl Skill for ColorContrast {
    fn name(&self) -> &'static str {
        "color_contrast"
    }
    fn description(&self) -> &'static str {
        "Compute the WCAG 2.1 contrast ratio between a foreground and background color and \
         report the pass/fail verdict at each WCAG level (AA / AAA for normal and large text). \
         The ratio is `(L1 + 0.05) / (L2 + 0.05)` where L1 / L2 are relative luminances per \
         WCAG 2.1 SC 1.4.3 (sRGB inverse-gamma + Rec. 709 luma weights). Output ranges 1.0 \
         (no contrast) to 21.0 (black on white)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ContrastArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ContrastArgs>()?;
            let (fr, fg, fb, _) = parse_color(&args.foreground)?;
            let (br, bg, bb, _) = parse_color(&args.background)?;
            let l_fg = relative_luminance(fr, fg, fb);
            let l_bg = relative_luminance(br, bg, bb);
            let (l1, l2) = if l_fg > l_bg {
                (l_fg, l_bg)
            } else {
                (l_bg, l_fg)
            };
            let ratio = (l1 + 0.05) / (l2 + 0.05);
            Ok(text_result(
                json!({
                    "foreground": args.foreground,
                    "background": args.background,
                    "contrast_ratio": round2(ratio),
                    "wcag": {
                        "aa_normal_text": ratio >= 4.5,
                        "aa_large_text": ratio >= 3.0,
                        "aaa_normal_text": ratio >= 7.0,
                        "aaa_large_text": ratio >= 4.5,
                    },
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Black on white (maximum)",
                args: r##"{"foreground": "#000", "background": "#fff"}"##,
                note: Some("Ratio 21.0, passes every WCAG level."),
            },
            SkillExample {
                title: "Common 'tasteful gray' pair (fails AA)",
                args: r##"{"foreground": "#777777", "background": "#ffffff"}"##,
                note: Some("Ratio ≈ 4.48 — JUST fails AA for normal text (≥4.5)."),
            },
            SkillExample {
                title: "Brand color check",
                args: r##"{"foreground": "#3498db", "background": "#ffffff"}"##,
                note: Some("Tells you which WCAG levels the pair passes / fails."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Audit a text + background pair against WCAG 2.1 AA / AAA before shipping a UI.",
            "Pick the threshold-meeting variant of a brand color for accessibility-critical text.",
            "Spot 'looks fine to me' contrast that actually fails AA.",
        ]
    }
}

// ---------------------------------------------------------------------------
// color_blend
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BlendArgs {
    /// First color (any form `color_convert` accepts).
    a: String,
    /// Second color (any form `color_convert` accepts).
    b: String,
    /// Blend factor: 0.0 = all `a`, 1.0 = all `b`. Defaults to 0.5
    /// (perceptual midpoint when `space="linear"`).
    #[serde(default)]
    t: Option<f64>,
    /// Blending color space: `linear` (sRGB inverse-gamma, perceptual, default)
    /// or `srgb` (naive sRGB-space lerp, the wrong thing but what most CSS tools do).
    #[serde(default)]
    space: Option<String>,
}

pub struct ColorBlend;
impl Skill for ColorBlend {
    fn name(&self) -> &'static str {
        "color_blend"
    }
    fn description(&self) -> &'static str {
        "Blend two colors at a factor `t` (0.0..1.0). Default space is `linear` (sRGB \
         inverse-gamma first, blend, then re-apply gamma) — this matches human perception. \
         `space=\"srgb\"` does the naive sRGB-space lerp that most CSS tools use, which is \
         visibly wrong but matches what designers see in their editors. Returns hex + rgb."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BlendArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<BlendArgs>()?;
            let (ar, ag, ab, _) = parse_color(&args.a)?;
            let (br, bg, bb, _) = parse_color(&args.b)?;
            let t = args.t.unwrap_or(0.5).clamp(0.0, 1.0);
            let space = args.space.as_deref().unwrap_or("linear").trim();
            let (r, g, b) = match space.to_ascii_lowercase().as_str() {
                "srgb" => (lerp(ar, br, t), lerp(ag, bg, t), lerp(ab, bb, t)),
                "linear" | "" => {
                    let (a_lr, a_lg, a_lb) =
                        (srgb_to_linear(ar), srgb_to_linear(ag), srgb_to_linear(ab));
                    let (b_lr, b_lg, b_lb) =
                        (srgb_to_linear(br), srgb_to_linear(bg), srgb_to_linear(bb));
                    let lr = lerp(a_lr, b_lr, t);
                    let lg = lerp(a_lg, b_lg, t);
                    let lb = lerp(a_lb, b_lb, t);
                    (linear_to_srgb(lr), linear_to_srgb(lg), linear_to_srgb(lb))
                }
                s => {
                    return Err(invalid(format!(
                        "unknown blend space `{s}` (try `linear` or `srgb`)"
                    )));
                }
            };
            Ok(text_result(
                json!({
                    "a": args.a,
                    "b": args.b,
                    "t": t,
                    "space": space,
                    "hex": format!("#{:02x}{:02x}{:02x}", to_u8(r), to_u8(g), to_u8(b)),
                    "rgb": [to_u8(r), to_u8(g), to_u8(b)],
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Perceptual midpoint of red and green",
                args: r##"{"a": "#ff0000", "b": "#00ff00"}"##,
                note: Some("`space=\"linear\"` (default) yields ~#bcbc00 — brighter than naive sRGB blending."),
            },
            SkillExample {
                title: "Naive sRGB blend for comparison",
                args: r##"{"a": "#ff0000", "b": "#00ff00", "space": "srgb"}"##,
                note: Some("Yields #808000 — matches CSS interpolation but is perceptually wrong."),
            },
            SkillExample {
                title: "75% toward the second color",
                args: r##"{"a": "#000000", "b": "#ffffff", "t": 0.75}"##,
                note: Some("Returns the perceptual 75-percentile gray."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute a true perceptual midpoint between two brand colors.",
            "Generate a gradient stop at a known position without HSL-twisting artifacts.",
            "Compare naive sRGB blending vs perceptually-correct linear blending.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Shared helpers — parsing
// ---------------------------------------------------------------------------

/// Parse any of the supported color forms into 0..1 RGB + alpha.
fn parse_color(s: &str) -> Result<(f64, f64, f64, f64), McpError> {
    let raw = s.trim();
    let lower = raw.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("rgb(")
        .or_else(|| lower.strip_prefix("rgba("))
    {
        let inside = rest
            .strip_suffix(')')
            .ok_or_else(|| invalid("unterminated rgb()"))?;
        let parts: Vec<&str> = inside.split(',').map(str::trim).collect();
        if !(3..=4).contains(&parts.len()) {
            return Err(invalid("rgb()/rgba() needs 3 or 4 components"));
        }
        let r = parts[0]
            .parse::<f64>()
            .map_err(|_| invalid("bad red component"))?
            / 255.0;
        let g = parts[1]
            .parse::<f64>()
            .map_err(|_| invalid("bad green component"))?
            / 255.0;
        let b = parts[2]
            .parse::<f64>()
            .map_err(|_| invalid("bad blue component"))?
            / 255.0;
        let a = if parts.len() == 4 {
            parts[3]
                .parse::<f64>()
                .map_err(|_| invalid("bad alpha component"))?
        } else {
            1.0
        };
        return Ok((r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a));
    }
    if let Some(rest) = lower
        .strip_prefix("hsl(")
        .or_else(|| lower.strip_prefix("hsla("))
    {
        let inside = rest
            .strip_suffix(')')
            .ok_or_else(|| invalid("unterminated hsl()"))?;
        let parts: Vec<&str> = inside.split(',').map(str::trim).collect();
        if !(3..=4).contains(&parts.len()) {
            return Err(invalid("hsl()/hsla() needs 3 or 4 components"));
        }
        let h: f64 = parts[0]
            .trim_end_matches("deg")
            .parse()
            .map_err(|_| invalid("bad hue"))?;
        let s_v: f64 = parts[1]
            .trim_end_matches('%')
            .parse::<f64>()
            .map_err(|_| invalid("bad saturation"))?
            / 100.0;
        let l_v: f64 = parts[2]
            .trim_end_matches('%')
            .parse::<f64>()
            .map_err(|_| invalid("bad lightness"))?
            / 100.0;
        let a = if parts.len() == 4 {
            parts[3]
                .parse::<f64>()
                .map_err(|_| invalid("bad alpha component"))?
        } else {
            1.0
        };
        let (r, g, b) = hsl_to_rgb(
            h.rem_euclid(360.0),
            s_v.clamp(0.0, 1.0),
            l_v.clamp(0.0, 1.0),
        );
        return Ok((r, g, b, a));
    }
    // Hex (with or without leading #).
    let hex = raw.trim_start_matches('#');
    let (r, g, b) = match hex.len() {
        3 => {
            let to = |c: char| -> Result<u8, McpError> {
                u8::from_str_radix(&c.to_string(), 16)
                    .map(|n| n | (n << 4))
                    .map_err(|_| invalid(format!("bad hex digit `{c}`")))
            };
            let bytes: Vec<char> = hex.chars().collect();
            (
                to(bytes[0])? as f64 / 255.0,
                to(bytes[1])? as f64 / 255.0,
                to(bytes[2])? as f64 / 255.0,
            )
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| invalid("bad hex (R)"))? as f64
                / 255.0;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| invalid("bad hex (G)"))? as f64
                / 255.0;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| invalid("bad hex (B)"))? as f64
                / 255.0;
            (r, g, b)
        }
        n => {
            return Err(invalid(format!(
                "unrecognized color `{s}` (got {n} hex chars; need 3 or 6, or use rgb()/hsl())"
            )));
        }
    };
    Ok((r, g, b, 1.0))
}

// ---------------------------------------------------------------------------
// Color-space math
// ---------------------------------------------------------------------------

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

fn to_u8(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// sRGB → linear (Rec. 709 inverse gamma).
fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// linear → sRGB.
fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// WCAG 2.1 relative luminance.
fn relative_luminance(r: f64, g: f64, b: f64) -> f64 {
    let (lr, lg, lb) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    0.2126 * lr + 0.7152 * lg + 0.0722 * lb
}

/// HSL → sRGB. Hue in degrees, saturation + lightness in 0..1.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_seg = h / 60.0;
    let x = c * (1.0 - (h_seg.rem_euclid(2.0) - 1.0).abs());
    let (r1, g1, b1) = match h_seg as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (r1 + m, g1 + m, b1 + m)
}

/// sRGB → HSL. Returns (h°, s, l) with s, l in 0..1.
fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-9 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        ((g - b) / d) + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        ((b - r) / d) + 2.0
    } else {
        ((r - g) / d) + 4.0
    };
    (h * 60.0, s, l)
}

/// sRGB → CIE L*a*b* (D65).
fn rgb_to_lab(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    // sRGB → linear → XYZ (D65).
    let (lr, lg, lb) = (srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b));
    let x = 0.4124564 * lr + 0.3575761 * lg + 0.1804375 * lb;
    let y = 0.2126729 * lr + 0.7151522 * lg + 0.0721750 * lb;
    let z = 0.0193339 * lr + 0.1191920 * lg + 0.9503041 * lb;
    // D65 reference white.
    let xn = 0.95047;
    let yn = 1.0;
    let zn = 1.08883;
    let fx = lab_f(x / xn);
    let fy = lab_f(y / yn);
    let fz = lab_f(z / zn);
    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let b = 200.0 * (fy - fz);
    (l, a, b)
}

fn lab_f(t: f64) -> f64 {
    if t > 0.008856 {
        t.cbrt()
    } else {
        7.787 * t + 16.0 / 116.0
    }
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "color"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Color tools: round-trip between hex / RGB / HSL / Lab, compute WCAG 2.1 contrast ratios, \
         blend two colors in linear or sRGB space. Pure local compute, no external deps. \
         Deterministic answers for LLM-typical hex-code / HSL-conversion tasks."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `color_convert { color: \"#3498db\" }` — what RGB / HSL / Lab is this?\n\
             2. `color_contrast { foreground: \"#3498db\", background: \"#ffffff\" }` — does it pass AA on white?\n\
             3. `color_blend { a: \"#3498db\", b: \"#ffffff\", t: 0.3 }` — find a lighter accessible variant.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ColorConvert),
        Box::new(ColorContrast),
        Box::new(ColorBlend),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_hex() {
        let (r, g, b, _) = parse_color("#abc").unwrap();
        assert!((r - 0xaa as f64 / 255.0).abs() < 1e-6);
        assert!((g - 0xbb as f64 / 255.0).abs() < 1e-6);
        assert!((b - 0xcc as f64 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn parses_rgb_call() {
        let (r, g, b, _) = parse_color("rgb(52, 152, 219)").unwrap();
        assert_eq!(to_u8(r), 52);
        assert_eq!(to_u8(g), 152);
        assert_eq!(to_u8(b), 219);
    }

    #[test]
    fn parses_hsl_pure_green() {
        let (r, g, b, _) = parse_color("hsl(120, 100%, 50%)").unwrap();
        assert_eq!(to_u8(r), 0);
        assert_eq!(to_u8(g), 255);
        assert_eq!(to_u8(b), 0);
    }

    #[test]
    fn contrast_black_on_white_is_21() {
        let l_fg = relative_luminance(0.0, 0.0, 0.0);
        let l_bg = relative_luminance(1.0, 1.0, 1.0);
        let ratio = (l_bg + 0.05) / (l_fg + 0.05);
        assert!((ratio - 21.0).abs() < 0.01, "got {ratio}");
    }

    #[test]
    fn gray_777_on_white_just_fails_aa() {
        let g = 0x77 as f64 / 255.0;
        let l_fg = relative_luminance(g, g, g);
        let l_bg = relative_luminance(1.0, 1.0, 1.0);
        let ratio = (l_bg + 0.05) / (l_fg + 0.05);
        // Documented AA threshold is 4.5; #777 on white is ~4.48 which fails.
        assert!(ratio < 4.5, "got {ratio}");
        assert!(ratio > 4.4, "got {ratio}");
    }

    #[test]
    fn hsl_roundtrip_through_rgb() {
        let (r, g, b) = hsl_to_rgb(204.0, 0.7, 0.53);
        let (h, s, l) = rgb_to_hsl(r, g, b);
        assert!((h - 204.0).abs() < 0.5, "got h={h}");
        assert!((s - 0.7).abs() < 0.02, "got s={s}");
        assert!((l - 0.53).abs() < 0.01, "got l={l}");
    }

    #[test]
    fn lab_pure_white_is_l100() {
        let (l, a, b) = rgb_to_lab(1.0, 1.0, 1.0);
        assert!((l - 100.0).abs() < 0.1, "got L={l}");
        assert!(a.abs() < 0.1, "got a={a}");
        assert!(b.abs() < 0.1, "got b={b}");
    }
}
