//! Specialty interchange formats — metadata-level readers for STL meshes
//! (binary + ASCII), MAVLink message-id resolver, GRIB2 section probe.
//! Read-only, lightweight; on by default.

use std::sync::Arc;

use anyhow::Result;
use base64::Engine;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StlArgs {
    /// Raw STL contents as base64 (binary STL) or as a UTF-8 string (ASCII STL).
    data_base64: Option<String>,
    /// ASCII STL contents as a UTF-8 string (alternative to `data_base64`).
    data_ascii: Option<String>,
}

pub struct InterchangeStlInfo;
impl Skill for InterchangeStlInfo {
    fn name(&self) -> &'static str {
        "interchange_stl_info"
    }
    fn description(&self) -> &'static str {
        "Parse an STL mesh (binary or ASCII) and return triangle count, \
        axis-aligned bounding box, surface area, and centroid. Handles \
        both binary and ASCII variants automatically."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StlArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<StlArgs>()?;
            let triangles = match (a.data_base64, a.data_ascii) {
                (Some(b64), None) => {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(b64.trim())
                        .map_err(|e| invalid(format!("base64: {e}")))?;
                    parse_stl_binary(&bytes)?
                }
                (None, Some(s)) => parse_stl_ascii(&s)?,
                _ => return Err(invalid("supply data_base64 OR data_ascii")),
            };
            if triangles.is_empty() {
                return Err(invalid("STL contains no triangles"));
            }
            let mut bbox = [
                f64::INFINITY,
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            ];
            let mut area = 0_f64;
            let mut centroid = [0_f64; 3];
            for t in &triangles {
                for v in &t.verts {
                    for i in 0..3 {
                        bbox[i] = bbox[i].min(v[i] as f64);
                        bbox[i + 3] = bbox[i + 3].max(v[i] as f64);
                        centroid[i] += v[i] as f64;
                    }
                }
                area += triangle_area(&t.verts);
            }
            let n = (triangles.len() * 3) as f64;
            for c in &mut centroid {
                *c /= n;
            }
            Ok(text_result(
                json!({
                    "triangle_count": triangles.len(),
                    "bbox_min": [bbox[0], bbox[1], bbox[2]],
                    "bbox_max": [bbox[3], bbox[4], bbox[5]],
                    "surface_area": area,
                    "centroid": centroid,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "ASCII STL inline",
                args: r#"{"data_ascii": "solid cube\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid\n"}"#,
                note: Some("Returns triangle count, bbox, surface area, and centroid as JSON."),
            },
            SkillExample {
                title: "Single-triangle ASCII STL",
                args: r#"{"data_ascii": "solid tri\nfacet normal 1 0 0\nouter loop\nvertex 0 0 0\nvertex 0 1 0\nvertex 0 0 1\nendloop\nendfacet\nendsolid\n"}"#,
                note: Some(
                    "Supply either `data_ascii` OR `data_base64`, not both. For binary STL pass \
                     the raw bytes base64-encoded (header is 84 bytes minimum + 50 per triangle).",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Quick mesh sanity check before importing into a CAD pipeline.",
            "Extract bbox / centroid for placing or scaling an STL model.",
            "Estimate surface area for printing-time / material calculations.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::ExactlyOne {
            fields: &["data_base64", "data_ascii"],
        }]
    }
}

struct StlTriangle {
    verts: [[f32; 3]; 3],
}

fn parse_stl_binary(bytes: &[u8]) -> std::result::Result<Vec<StlTriangle>, McpError> {
    if bytes.len() < 84 {
        return Err(invalid("binary STL too short"));
    }
    let n = u32::from_le_bytes(bytes[80..84].try_into().unwrap()) as usize;
    if bytes.len() < 84 + n * 50 {
        return Err(invalid("binary STL truncated"));
    }
    let mut out = Vec::with_capacity(n);
    let mut off = 84;
    for _ in 0..n {
        // Skip normal (12 bytes).
        off += 12;
        let mut verts = [[0_f32; 3]; 3];
        for v in &mut verts {
            for c in v.iter_mut() {
                *c = f32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
                off += 4;
            }
        }
        off += 2; // attribute byte count
        out.push(StlTriangle { verts });
    }
    Ok(out)
}

fn parse_stl_ascii(s: &str) -> std::result::Result<Vec<StlTriangle>, McpError> {
    let mut out = Vec::new();
    let mut current = Vec::new();
    for line in s.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("vertex ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() < 3 {
                return Err(invalid("ASCII STL: malformed vertex"));
            }
            let v = [
                parts[0]
                    .parse::<f32>()
                    .map_err(|e| invalid(format!("{e}")))?,
                parts[1]
                    .parse::<f32>()
                    .map_err(|e| invalid(format!("{e}")))?,
                parts[2]
                    .parse::<f32>()
                    .map_err(|e| invalid(format!("{e}")))?,
            ];
            current.push(v);
            if current.len() == 3 {
                out.push(StlTriangle {
                    verts: [current[0], current[1], current[2]],
                });
                current.clear();
            }
        }
    }
    Ok(out)
}

fn triangle_area(v: &[[f32; 3]; 3]) -> f64 {
    let a = [
        (v[1][0] - v[0][0]) as f64,
        (v[1][1] - v[0][1]) as f64,
        (v[1][2] - v[0][2]) as f64,
    ];
    let b = [
        (v[2][0] - v[0][0]) as f64,
        (v[2][1] - v[0][1]) as f64,
        (v[2][2] - v[0][2]) as f64,
    ];
    let cross = [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ];
    0.5 * (cross[0].powi(2) + cross[1].powi(2) + cross[2].powi(2)).sqrt()
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(InterchangeStlInfo)]
}
