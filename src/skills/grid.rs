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

/// Descriptive User-Agent for OSM/Overpass calls. **Required** — Apache at
/// `overpass-api.de` returns 406 for browser-like UAs and for `curl/*`. OSM's
/// UA policy asks third-party clients to identify themselves.
pub(crate) const OVERPASS_UA: &str = crate::LODESTONE_UA;

async fn run_overpass(server: &crate::Lodestone, query: &str) -> Result<Value, McpError> {
    let cache_key = format!("grid_overpass|{}", crate::constellation::hash_key(query));
    if let Some(c) = server.retrieval_get(&cache_key).await {
        if let Ok(v) = serde_json::from_str::<Value>(&c) {
            return Ok(v);
        }
    }
    let r = server
        .http
        .post("https://overpass-api.de/api/interpreter")
        .body(format!("data={}", url_encode(query)))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Accept", "application/json")
        .header("User-Agent", OVERPASS_UA)
        .send()
        .await
        .and_then(|x| x.error_for_status())
        .map_err(|e| internal(anyhow::anyhow!("overpass: {e}")))?;
    let v: Value = r
        .json()
        .await
        .map_err(|e| internal(anyhow::anyhow!("overpass parse: {e}")))?;
    if let Ok(s) = serde_json::to_string(&v) {
        server.retrieval_put(cache_key, &s);
    }
    Ok(v)
}

// ----- QL builders (extracted for unit tests) -----

fn bbox_str(south: f64, west: f64, north: f64, east: f64) -> String {
    format!("({south},{west},{north},{east})")
}

pub(crate) fn power_plant_ql(
    south: f64,
    west: f64,
    north: f64,
    east: f64,
    source: Option<&str>,
) -> String {
    let bbox = bbox_str(south, west, north, east);
    let s = source
        .map(|x| format!(r#"["plant:source"="{}"]"#, x.trim().to_ascii_lowercase()))
        .unwrap_or_default();
    format!(
        "[out:json][timeout:60];(node[\"power\"=\"plant\"]{s}{bbox};way[\"power\"=\"plant\"]{s}{bbox};relation[\"power\"=\"plant\"]{s}{bbox};);out center tags;"
    )
}

pub(crate) fn substation_ql(south: f64, west: f64, north: f64, east: f64) -> String {
    let bbox = bbox_str(south, west, north, east);
    format!(
        "[out:json][timeout:60];(node[\"power\"=\"substation\"]{bbox};way[\"power\"=\"substation\"]{bbox};relation[\"power\"=\"substation\"]{bbox};);out center tags;"
    )
}

pub(crate) fn transmission_ql(south: f64, west: f64, north: f64, east: f64) -> String {
    let bbox = bbox_str(south, west, north, east);
    format!(
        "[out:json][timeout:60];(way[\"power\"~\"^(line|minor_line)$\"]{bbox};);out center tags;"
    )
}

pub(crate) fn data_center_ql(south: f64, west: f64, north: f64, east: f64) -> String {
    let bbox = bbox_str(south, west, north, east);
    format!(
        "[out:json][timeout:60];(node[\"telecom\"=\"data_center\"]{bbox};way[\"telecom\"=\"data_center\"]{bbox};node[\"building\"=\"data_center\"]{bbox};way[\"building\"=\"data_center\"]{bbox};);out center tags;"
    )
}

pub(crate) fn pipeline_ql(south: f64, west: f64, north: f64, east: f64, substance: &str) -> String {
    let bbox = bbox_str(south, west, north, east);
    format!(
        "[out:json][timeout:60];(way[\"man_made\"=\"pipeline\"][\"substance\"=\"{substance}\"]{bbox};);out center tags;"
    )
}

pub(crate) fn submarine_cable_ql(south: f64, west: f64, north: f64, east: f64) -> String {
    let bbox = bbox_str(south, west, north, east);
    format!(
        "[out:json][timeout:60];(way[\"submarine\"=\"yes\"][\"communication\"=\"line\"]{bbox};way[\"submarine\"=\"yes\"][\"power\"=\"cable\"]{bbox};relation[\"route\"=\"submarine_cable\"]{bbox};);out center tags;"
    )
}

pub(crate) fn flood_ql(south: f64, west: f64, north: f64, east: f64) -> String {
    let bbox = bbox_str(south, west, north, east);
    format!(
        "[out:json][timeout:60];(way[\"natural\"=\"floodway\"]{bbox};relation[\"natural\"=\"floodway\"]{bbox};way[\"hazard\"=\"flood_prone\"]{bbox};relation[\"hazard\"=\"flood_prone\"]{bbox};way[\"hazard\"=\"flood\"]{bbox};way[\"hazard:type\"=\"flood\"]{bbox};way[\"landuse\"=\"basin\"][\"basin\"~\"detention|flood|retention\"]{bbox};relation[\"landuse\"=\"basin\"][\"basin\"~\"detention|flood|retention\"]{bbox};);out center tags;"
    )
}

pub(crate) fn planned_lines_ql(south: f64, west: f64, north: f64, east: f64) -> String {
    let bbox = bbox_str(south, west, north, east);
    format!(
        "[out:json][timeout:60];(way[\"proposed:power\"~\"^(line|minor_line)$\"]{bbox};way[\"construction:power\"~\"^(line|minor_line)$\"]{bbox};way[\"power\"~\"^(line|minor_line)$\"][\"construction\"=\"yes\"]{bbox};way[\"power\"~\"^(line|minor_line)$\"][\"proposed\"=\"yes\"]{bbox};);out center tags;"
    )
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
            let ql = power_plant_ql(
                args.south,
                args.west,
                args.north,
                args.east,
                args.source.as_deref(),
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
            let ql = substation_ql(args.south, args.west, args.north, args.east);
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
            let ql = transmission_ql(args.south, args.west, args.north, args.east);
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
            let ql = data_center_ql(args.south, args.west, args.north, args.east);
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
            let ql = pipeline_ql(args.south, args.west, args.north, args.east, &substance);
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
            let ql = submarine_cable_ql(args.south, args.west, args.north, args.east);
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

// ----- grid_flood_zones -----

pub struct GridFloodZones;
impl Skill for GridFloodZones {
    fn name(&self) -> &'static str {
        "grid_flood_zones"
    }
    fn description(&self) -> &'static str {
        "Find tagged flood hazard / floodway / detention features in a bounding box (OSM \
        `natural=floodway`, `hazard=flood`/`flood_prone`, `landuse=basin`+`basin=detention`/\
        `flood`). OSM coverage is uneven outside well-mapped regions — for authoritative \
        US floodplains use FEMA NFHL separately; this is the openly tagged subset."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BboxArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<BboxArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let ql = flood_ql(args.south, args.west, args.north, args.east);
            let v = run_overpass(server, &ql).await?;
            let empty = Vec::new();
            let elements = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            Ok(text_result(render_elements(
                elements,
                "Flood hazard / detention features",
                max,
                args.name_filter.as_deref(),
                &["natural", "hazard", "hazard:type", "basin", "landuse"],
            )))
        })
    }
}

// ----- grid_planned_lines -----

pub struct GridPlannedLines;
impl Skill for GridPlannedLines {
    fn name(&self) -> &'static str {
        "grid_planned_lines"
    }
    fn description(&self) -> &'static str {
        "Find PLANNED / UNDER-CONSTRUCTION transmission lines in a bounding box (OSM \
        `proposed:power=line`/`minor_line` and `construction:power=line`/`minor_line`). For \
        authoritative European TYNDP projects, ENTSO-E publishes separately (not covered here \
        because their API needs a key)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TransmissionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<TransmissionArgs>()?;
            check_bbox(args.south, args.west, args.north, args.east)?;
            let max = args.max.unwrap_or(100).clamp(1, 1000) as usize;
            let ql = planned_lines_ql(args.south, args.west, args.north, args.east);
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
            let owned: Vec<Value> = elements.into_iter().cloned().collect();
            Ok(text_result(render_elements(
                &owned,
                "Planned / under-construction transmission lines",
                max,
                args.name_filter.as_deref(),
                &[
                    "voltage",
                    "circuits",
                    "operator",
                    "construction",
                    "proposed",
                ],
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
        Box::new(GridFloodZones),
        Box::new(GridPlannedLines),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // A small bbox around Redmond, WA — gives the QL strings a stable shape.
    const SOUTH: f64 = 47.5;
    const WEST: f64 = -122.3;
    const NORTH: f64 = 47.7;
    const EAST: f64 = -122.1;

    #[test]
    fn ua_is_descriptive_and_not_browser_like() {
        // overpass-api.de 406s requests whose User-Agent is browser-like or `curl/*`.
        // The lodestone OVERPASS_UA must look like an application, not a browser or
        // a CLI tool, to pass Apache's content negotiation.
        assert!(
            OVERPASS_UA.contains("lodestone-mcp"),
            "UA must identify the application"
        );
        for forbidden in ["Mozilla", "Chrome", "AppleWebKit", "curl/"] {
            assert!(
                !OVERPASS_UA.contains(forbidden),
                "UA must not look like `{forbidden}` (Apache will 406)"
            );
        }
        // OSM UA policy: include a contact / URL.
        assert!(
            OVERPASS_UA.contains("http"),
            "UA should embed a contact URL per OSM's UA policy"
        );
    }

    #[test]
    fn power_plant_ql_shape() {
        let ql = power_plant_ql(SOUTH, WEST, NORTH, EAST, None);
        assert!(ql.starts_with("[out:json]"));
        assert!(ql.contains("[\"power\"=\"plant\"]"));
        assert!(ql.contains("(47.5,-122.3,47.7,-122.1)"));
        // All three element kinds queried so a single Overpass call covers a node-
        // tagged plant, a way-tagged building, and a relation-tagged multi-unit site.
        for kind in ["node", "way", "relation"] {
            assert!(ql.contains(&format!("{kind}[\"power\"=\"plant\"]")));
        }
        assert!(ql.ends_with("out center tags;"));
        // No backslash-continuation whitespace leaks (those broke QL syntax in
        // earlier drafts).
        assert!(
            !ql.contains("  "),
            "QL must not contain double spaces; got: {ql}"
        );
    }

    #[test]
    fn power_plant_ql_source_filter() {
        let ql = power_plant_ql(SOUTH, WEST, NORTH, EAST, Some("nuclear"));
        assert!(ql.contains("[\"plant:source\"=\"nuclear\"]"));
        // Filter is lowercased.
        let ql = power_plant_ql(SOUTH, WEST, NORTH, EAST, Some("  Solar  "));
        assert!(ql.contains("[\"plant:source\"=\"solar\"]"));
        // Without a filter, the predicate is absent.
        let ql = power_plant_ql(SOUTH, WEST, NORTH, EAST, None);
        assert!(!ql.contains("plant:source"));
    }

    #[test]
    fn substation_and_transmission_ql_shape() {
        let s = substation_ql(SOUTH, WEST, NORTH, EAST);
        assert!(s.contains("[\"power\"=\"substation\"]"));
        for kind in ["node", "way", "relation"] {
            assert!(s.contains(kind));
        }
        let t = transmission_ql(SOUTH, WEST, NORTH, EAST);
        assert!(t.contains("[\"power\"~\"^(line|minor_line)$\"]"));
        // Transmission queries `way` only; nodes for a line don't make sense.
        assert!(!t.contains("node["));
    }

    #[test]
    fn pipeline_ql_carries_substance() {
        let ql = pipeline_ql(SOUTH, WEST, NORTH, EAST, "hydrogen");
        assert!(ql.contains("[\"man_made\"=\"pipeline\"]"));
        assert!(ql.contains("[\"substance\"=\"hydrogen\"]"));
    }

    #[test]
    fn submarine_cable_and_data_center_ql() {
        let c = submarine_cable_ql(SOUTH, WEST, NORTH, EAST);
        assert!(c.contains("[\"submarine\"=\"yes\"][\"communication\"=\"line\"]"));
        assert!(c.contains("[\"route\"=\"submarine_cable\"]"));
        let d = data_center_ql(SOUTH, WEST, NORTH, EAST);
        assert!(d.contains("[\"telecom\"=\"data_center\"]"));
        assert!(d.contains("[\"building\"=\"data_center\"]"));
    }

    #[test]
    fn flood_and_planned_lines_ql() {
        let f = flood_ql(SOUTH, WEST, NORTH, EAST);
        for tag in [
            "[\"natural\"=\"floodway\"]",
            "[\"hazard\"=\"flood_prone\"]",
            "[\"hazard\"=\"flood\"]",
            "[\"hazard:type\"=\"flood\"]",
            "[\"basin\"~\"detention|flood|retention\"]",
        ] {
            assert!(f.contains(tag), "missing {tag}");
        }
        let p = planned_lines_ql(SOUTH, WEST, NORTH, EAST);
        assert!(p.contains("[\"proposed:power\"~\"^(line|minor_line)$\"]"));
        assert!(p.contains("[\"construction:power\"~\"^(line|minor_line)$\"]"));
    }

    /// Live integration test against the public Overpass server — pinned to the
    /// **smallest possible** query (one node, point bbox) so it returns fast and
    /// doesn't hit the rate limiter. This is the test that catches the original
    /// 406 bug: if the User-Agent or Accept header is wrong, `error_for_status`
    /// fires and the test fails.
    ///
    /// Marked `#[ignore]` because it needs the network. Run explicitly with:
    /// `cargo test --bin lodestone-mcp -- --ignored grid::tests::overpass_live`.
    #[tokio::test]
    #[ignore]
    async fn overpass_live_returns_json_with_proper_ua() {
        let http = reqwest::Client::builder()
            // Deliberately mirror the production client's UA quirks.
            .user_agent(crate::LODESTONE_UA)
            .build()
            .unwrap();
        // Tiny query: count Redmond's substations within a 0.02° box.
        let ql = substation_ql(47.66, -122.13, 47.68, -122.11);
        let body = format!("data={}", url_encode(&ql));
        let resp = http
            .post("https://overpass-api.de/api/interpreter")
            .body(body)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .header("User-Agent", OVERPASS_UA)
            .send()
            .await
            .expect("network failure");
        assert!(
            resp.status().is_success(),
            "Overpass returned {} — UA/Accept rejection regression",
            resp.status()
        );
        let json: Value = resp.json().await.expect("body wasn't JSON");
        assert_eq!(
            json.get("version").and_then(|v| v.as_f64()),
            Some(0.6),
            "expected Overpass JSON envelope"
        );
    }
}
