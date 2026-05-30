//! Power-grid + critical-infrastructure skills — the layers OpenGridWorks
//! visualizes (power plants, transmission lines, substations, data centres,
//! gas pipelines, submarine cables), built as thin typed wrappers over
//! `osm_overpass`. Keyless; all data comes from OpenStreetMap's Overpass API.
//!
//! For arbitrary tag queries use `osm_overpass` directly; these tools just
//! preformulate the QL so the model doesn't have to.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BboxArgs {
    /// South latitude of the bounding box.
    south: f64,
    /// West longitude of the bounding box.
    west: f64,
    /// North latitude of the bounding box.
    north: f64,
    /// East longitude of the bounding box.
    east: f64,
    /// Max elements to summarize (default 100, capped at 1000).
    #[serde(default)]
    max: Option<u32>,
    /// Optional name substring to filter results (case-insensitive).
    #[serde(default)]
    name_filter: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PowerPlantArgs {
    south: f64,
    west: f64,
    north: f64,
    east: f64,
    /// Filter by `plant:source` (coal, gas, oil, nuclear, wind, solar, hydro,
    /// biomass, geothermal). Omit for any.
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    max: Option<u32>,
    #[serde(default)]
    name_filter: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PipelineArgs {
    south: f64,
    west: f64,
    north: f64,
    east: f64,
    /// Pipeline `substance` (gas, oil, water, hydrogen, …). Default "gas".
    #[serde(default)]
    substance: Option<String>,
    #[serde(default)]
    max: Option<u32>,
    #[serde(default)]
    name_filter: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TransmissionArgs {
    south: f64,
    west: f64,
    north: f64,
    east: f64,
    /// Optional minimum operating voltage in volts (e.g. 110000 for ≥ 110 kV HV).
    /// OSM tags voltage as a string of volts; this filter is a soft hint applied
    /// to the result rows when present.
    #[serde(default)]
    min_voltage_v: Option<u64>,
    #[serde(default)]
    max: Option<u32>,
    #[serde(default)]
    name_filter: Option<String>,
}

fn check_bbox(south: f64, west: f64, north: f64, east: f64) -> Result<(), McpError> {
    if !(south.is_finite() && west.is_finite() && north.is_finite() && east.is_finite()) {
        return Err(invalid("bbox values must be finite"));
    }
    if south >= north || west >= east {
        return Err(invalid("bbox must have south < north and west < east"));
    }
    Ok(())
}

async fn run_overpass(server: &crate::Lodestone, query: &str) -> Result<Value, McpError> {
    let r = server
        .http
        .post("https://overpass-api.de/api/interpreter")
        .body(format!("data={}", url_encode(query)))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send()
        .await
        .and_then(|x| x.error_for_status())
        .map_err(|e| internal(anyhow::anyhow!("overpass: {e}")))?;
    r.json()
        .await
        .map_err(|e| internal(anyhow::anyhow!("overpass parse: {e}")))
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Pull (type, id, lat, lon, name) plus a tags handle for an element row.
fn element_summary(el: &Value) -> (String, i64, Option<f64>, Option<f64>, String, Value) {
    let typ = el
        .get("type")
        .and_then(|x| x.as_str())
        .unwrap_or("?")
        .to_string();
    let id = el.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
    let lat = el.get("lat").and_then(|x| x.as_f64()).or_else(|| {
        el.get("center")
            .and_then(|c| c.get("lat"))
            .and_then(|x| x.as_f64())
    });
    let lon = el.get("lon").and_then(|x| x.as_f64()).or_else(|| {
        el.get("center")
            .and_then(|c| c.get("lon"))
            .and_then(|x| x.as_f64())
    });
    let tags = el.get("tags").cloned().unwrap_or(Value::Null);
    let name = tags
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    (typ, id, lat, lon, name, tags)
}

fn render_elements(
    elements: &[Value],
    title: &str,
    max: usize,
    name_filter: Option<&str>,
    extra_tags: &[&str],
) -> String {
    let filter_lc = name_filter.map(|s| s.to_ascii_lowercase());
    let mut out = format!("{title}: {} elements", elements.len());
    let filtered: Vec<&Value> = elements
        .iter()
        .filter(|el| {
            if let Some(f) = &filter_lc {
                let (_, _, _, _, name, tags) = element_summary(el);
                let label = name.to_ascii_lowercase();
                let operator = tags
                    .get("operator")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                label.contains(f) || operator.contains(f)
            } else {
                true
            }
        })
        .collect();
    if name_filter.is_some() {
        out.push_str(&format!(" ({} match filter)", filtered.len()));
    }
    out.push_str(&format!(" — showing up to {max}:\n"));
    for el in filtered.iter().take(max) {
        let (typ, id, lat, lon, name, tags) = element_summary(el);
        let coords = match (lat, lon) {
            (Some(a), Some(b)) => format!(" ({a:.5}, {b:.5})"),
            _ => String::new(),
        };
        let label = if name.is_empty() {
            "(unnamed)".to_string()
        } else {
            name
        };
        let mut extras: Vec<String> = Vec::new();
        for k in extra_tags {
            if let Some(v) = tags.get(*k).and_then(|x| x.as_str()) {
                if !v.is_empty() {
                    extras.push(format!("{k}={v}"));
                }
            }
        }
        let extra_s = if extras.is_empty() {
            String::new()
        } else {
            format!("  · {}", extras.join(" · "))
        };
        out.push_str(&format!("  {typ}/{id}{coords}  {label}{extra_s}\n"));
    }
    out
}

// ----- grid_power_plants -----

pub struct GridPowerPlants;
impl Skill for GridPowerPlants {
    fn name(&self) -> &'static str {
        "grid_power_plants"
    }
    fn description(&self) -> &'static str {
        "Find power plants in a bounding box (OSM `power=plant`), optionally filtered by \
        `source` (coal/gas/oil/nuclear/wind/solar/hydro/biomass/geothermal). Returns name, \
        operator, source, and capacity (MW) where tagged."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PowerPlantArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PowerPlantArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let source_filter = args
                .source
                .as_ref()
                .map(|s| format!(r#"["plant:source"="{}"]"#, s.trim().to_ascii_lowercase()))
                .unwrap_or_default();
            let bbox = format!(
                "({},{},{},{})",
                args.south, args.west, args.north, args.east
            );
            let ql = format!(
                "[out:json][timeout:60];\
                 (node[\"power\"=\"plant\"]{source_filter}{bbox};\
                  way[\"power\"=\"plant\"]{source_filter}{bbox};\
                  relation[\"power\"=\"plant\"]{source_filter}{bbox};);\
                 out center tags;"
            );
            let v = run_overpass(server, &ql).await?;
            let empty = Vec::new();
            let elements = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            Ok(text_result(render_elements(
                elements,
                "Power plants",
                max,
                args.name_filter.as_deref(),
                &["plant:source", "plant:output:electricity", "operator"],
            )))
        })
    }
}

// ----- grid_substations -----

pub struct GridSubstations;
impl Skill for GridSubstations {
    fn name(&self) -> &'static str {
        "grid_substations"
    }
    fn description(&self) -> &'static str {
        "Find electrical substations in a bounding box (OSM `power=substation`). Returns name, \
        operator, and voltage where tagged."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BboxArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<BboxArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let bbox = format!(
                "({},{},{},{})",
                args.south, args.west, args.north, args.east
            );
            let ql = format!(
                "[out:json][timeout:60];\
                 (node[\"power\"=\"substation\"]{bbox};\
                  way[\"power\"=\"substation\"]{bbox};\
                  relation[\"power\"=\"substation\"]{bbox};);\
                 out center tags;"
            );
            let v = run_overpass(server, &ql).await?;
            let empty = Vec::new();
            let elements = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            Ok(text_result(render_elements(
                elements,
                "Substations",
                max,
                args.name_filter.as_deref(),
                &["voltage", "operator", "substation"],
            )))
        })
    }
}

// ----- grid_transmission_lines -----

pub struct GridTransmissionLines;
impl Skill for GridTransmissionLines {
    fn name(&self) -> &'static str {
        "grid_transmission_lines"
    }
    fn description(&self) -> &'static str {
        "Find high-voltage transmission lines in a bounding box (OSM `power=line`/`minor_line`). \
        Optional `min_voltage_v` filters to lines whose tagged voltage is ≥ that (in volts; \
        e.g. 110000 for ≥ 110 kV). Returns voltage, owner/operator, and circuits where tagged."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TransmissionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<TransmissionArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let bbox = format!(
                "({},{},{},{})",
                args.south, args.west, args.north, args.east
            );
            let ql = format!(
                "[out:json][timeout:60];\
                 (way[\"power\"~\"^(line|minor_line)$\"]{bbox};);\
                 out center tags;"
            );
            let v = run_overpass(server, &ql).await?;
            let empty = Vec::new();
            let mut elements: Vec<&Value> = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty)
                .iter()
                .collect();
            if let Some(min_v) = args.min_voltage_v {
                elements.retain(|el| {
                    el.get("tags")
                        .and_then(|t| t.get("voltage"))
                        .and_then(|x| x.as_str())
                        .and_then(|s| {
                            s.split(';')
                                .filter_map(|p| p.trim().parse::<u64>().ok())
                                .max()
                        })
                        .is_some_and(|v| v >= min_v)
                });
            }
            // re-emit as Vec<Value> for the renderer.
            let owned: Vec<Value> = elements.into_iter().cloned().collect();
            Ok(text_result(render_elements(
                &owned,
                "Transmission lines",
                max,
                args.name_filter.as_deref(),
                &["voltage", "circuits", "operator", "cables"],
            )))
        })
    }
}

// ----- grid_data_centers -----

pub struct GridDataCenters;
impl Skill for GridDataCenters {
    fn name(&self) -> &'static str {
        "grid_data_centers"
    }
    fn description(&self) -> &'static str {
        "Find data centres in a bounding box (OSM `telecom=data_center` / `building=data_center`)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BboxArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<BboxArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let bbox = format!(
                "({},{},{},{})",
                args.south, args.west, args.north, args.east
            );
            let ql = format!(
                "[out:json][timeout:60];\
                 (node[\"telecom\"=\"data_center\"]{bbox};\
                  way[\"telecom\"=\"data_center\"]{bbox};\
                  node[\"building\"=\"data_center\"]{bbox};\
                  way[\"building\"=\"data_center\"]{bbox};);\
                 out center tags;"
            );
            let v = run_overpass(server, &ql).await?;
            let empty = Vec::new();
            let elements = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            Ok(text_result(render_elements(
                elements,
                "Data centres",
                max,
                args.name_filter.as_deref(),
                &["operator", "telecom", "building"],
            )))
        })
    }
}

// ----- grid_pipelines -----

pub struct GridPipelines;
impl Skill for GridPipelines {
    fn name(&self) -> &'static str {
        "grid_pipelines"
    }
    fn description(&self) -> &'static str {
        "Find pipelines in a bounding box (OSM `man_made=pipeline`), filtered by `substance` \
        (default gas — others: oil, water, hydrogen, sewage, …). Returns name, operator, \
        substance, and diameter where tagged."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PipelineArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PipelineArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let substance = args
                .substance
                .as_ref()
                .map(|s| s.trim().to_ascii_lowercase())
                .unwrap_or_else(|| "gas".to_string());
            let bbox = format!(
                "({},{},{},{})",
                args.south, args.west, args.north, args.east
            );
            let ql = format!(
                "[out:json][timeout:60];\
                 (way[\"man_made\"=\"pipeline\"][\"substance\"=\"{substance}\"]{bbox};);\
                 out center tags;"
            );
            let v = run_overpass(server, &ql).await?;
            let empty = Vec::new();
            let elements = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            Ok(text_result(render_elements(
                elements,
                &format!("Pipelines (substance={substance})"),
                max,
                args.name_filter.as_deref(),
                &["substance", "operator", "diameter", "location"],
            )))
        })
    }
}

// ----- grid_submarine_cables -----

pub struct GridSubmarineCables;
impl Skill for GridSubmarineCables {
    fn name(&self) -> &'static str {
        "grid_submarine_cables"
    }
    fn description(&self) -> &'static str {
        "Find submarine communications cables intersecting a bounding box (OSM \
        `submarine=yes` + `communication=line`/`route=submarine_cable`). Returns name, \
        operator, and landing points where tagged. Use a WIDE bbox — cables span oceans."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BboxArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<BboxArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let bbox = format!(
                "({},{},{},{})",
                args.south, args.west, args.north, args.east
            );
            let ql = format!(
                "[out:json][timeout:60];\
                 (way[\"submarine\"=\"yes\"][\"communication\"=\"line\"]{bbox};\
                  way[\"submarine\"=\"yes\"][\"power\"=\"cable\"]{bbox};\
                  relation[\"route\"=\"submarine_cable\"]{bbox};);\
                 out center tags;"
            );
            let v = run_overpass(server, &ql).await?;
            let empty = Vec::new();
            let elements = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            Ok(text_result(render_elements(
                elements,
                "Submarine cables",
                max,
                args.name_filter.as_deref(),
                &["operator", "owner", "communication", "power"],
            )))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(GridPowerPlants),
        Box::new(GridSubstations),
        Box::new(GridTransmissionLines),
        Box::new(GridDataCenters),
        Box::new(GridPipelines),
        Box::new(GridSubmarineCables),
    ]
}
