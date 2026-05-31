//! OpenStreetMap and GIS skills — keyless public APIs for
//! geocoding/reverse-geocoding (Nominatim), arbitrary feature queries
//! (Overpass), elevation lookups (Open-Elevation), and routing (OSRM public
//! demo). Plus local GIS helpers (bbox, point-in-polygon, GeoJSON summary).
//!
//! All keyless. Cached through the retrieval cache. Public APIs ask for a
//! User-Agent and reasonable rate; we use the shared `USER_AGENT` and respect
//! their usage policies. For distance/bearing between two coordinates, use the
//! existing `geo_distance` / `geo_azimuth` tools in the `geometry` skill.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, send_json_ctx, Skill, SkillCtx};
use crate::{invalid, text_result};

// Not gated by config — keyless public APIs, always on (like wikipedia/arxiv).

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

async fn http_json(server: &crate::Lodestone, url: &str) -> Result<Value, McpError> {
    send_json_ctx(
        server.http.get(url).header("Accept", "application/json"),
        &format!("HTTP {url}"),
    )
    .await
}

// ----- osm_geocode -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GeocodeArgs {
    /// Free-form place name or address ("Redmond, WA", "Eiffel Tower", "Berlin Hbf").
    query: String,
    /// Max results (default 5, capped at 25).
    #[serde(default)]
    max: Option<u32>,
}

pub struct OsmGeocode;
impl Skill for OsmGeocode {
    fn name(&self) -> &'static str {
        "osm_geocode"
    }
    fn description(&self) -> &'static str {
        "Forward-geocode a place name or address via OpenStreetMap Nominatim (keyless). Returns \
        ranked candidates with display name, lat/lon, place type, and OSM id. Pair with \
        `geo_distance` / `osm_route` to chain into distance or routing."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GeocodeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<GeocodeArgs>()?;
            let limit = args.max.unwrap_or(5).clamp(1, 25);
            let cache = format!("osm_geocode|{limit}|{}", args.query.trim());
            if let Some(c) = server.retrieval_get(&cache).await {
                return Ok(text_result(c));
            }
            let url = format!(
                "https://nominatim.openstreetmap.org/search?q={}&format=json&addressdetails=1&limit={limit}",
                url_encode(args.query.trim())
            );
            let v = http_json(server, &url).await?;
            let empty = Vec::new();
            let arr = v.as_array().unwrap_or(&empty);
            if arr.is_empty() {
                return Ok(text_result(format!("No matches for: {}", args.query)));
            }
            let mut out = format!("{} match(es) for \"{}\":\n", arr.len(), args.query);
            for (i, item) in arr.iter().enumerate() {
                let name = item
                    .get("display_name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("(no name)");
                let lat = item.get("lat").and_then(|x| x.as_str()).unwrap_or("");
                let lon = item.get("lon").and_then(|x| x.as_str()).unwrap_or("");
                let class = item.get("class").and_then(|x| x.as_str()).unwrap_or("");
                let typ = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
                let osm_type = item.get("osm_type").and_then(|x| x.as_str()).unwrap_or("");
                let osm_id = item.get("osm_id").and_then(|x| x.as_i64()).unwrap_or(0);
                out.push_str(&format!(
                    "\n{}. {name}\n   lat {lat}, lon {lon}  · {class}/{typ}  · OSM {osm_type}/{osm_id}\n",
                    i + 1
                ));
            }
            server.retrieval_put(cache, &out);
            Ok(text_result(out))
        })
    }
}

// ----- osm_reverse_geocode -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReverseArgs {
    lat: f64,
    lon: f64,
    /// Detail zoom 0..18 (default 18 = building level).
    #[serde(default)]
    zoom: Option<u32>,
}

pub struct OsmReverseGeocode;
impl Skill for OsmReverseGeocode {
    fn name(&self) -> &'static str {
        "osm_reverse_geocode"
    }
    fn description(&self) -> &'static str {
        "Reverse-geocode a lat/lon to the nearest named place / address via Nominatim. Returns \
        display name and component fields (road, city, state, postcode, country)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ReverseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ReverseArgs>()?;
            let zoom = args.zoom.unwrap_or(18).clamp(0, 18);
            let cache = format!("osm_reverse|{:.6}|{:.6}|{zoom}", args.lat, args.lon);
            if let Some(c) = server.retrieval_get(&cache).await {
                return Ok(text_result(c));
            }
            let url = format!(
                "https://nominatim.openstreetmap.org/reverse?lat={}&lon={}&format=json&zoom={zoom}",
                args.lat, args.lon
            );
            let v = http_json(server, &url).await?;
            let name = v
                .get("display_name")
                .and_then(|x| x.as_str())
                .unwrap_or("(no name)");
            let mut out = format!("At ({:.6}, {:.6}):\n  {name}\n", args.lat, args.lon);
            if let Some(addr) = v.get("address").and_then(|x| x.as_object()) {
                let interesting = [
                    "road",
                    "suburb",
                    "neighbourhood",
                    "city",
                    "town",
                    "village",
                    "county",
                    "state",
                    "postcode",
                    "country",
                    "country_code",
                ];
                for k in interesting {
                    if let Some(v) = addr.get(k).and_then(|x| x.as_str()) {
                        out.push_str(&format!("  {k:<14}  {v}\n"));
                    }
                }
            }
            server.retrieval_put(cache, &out);
            Ok(text_result(out))
        })
    }
}

// ----- osm_overpass -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OverpassArgs {
    /// Overpass QL query, e.g. `[out:json][timeout:25];node["amenity"="pharmacy"](47.6,-122.2,47.7,-122.1);out;`.
    /// Always include `[out:json]`; bound the bbox tightly or you'll be rate-limited.
    query: String,
    /// Max elements to summarize in the output (default 50, capped at 500).
    #[serde(default)]
    max: Option<u32>,
}

pub struct OsmOverpass;
impl Skill for OsmOverpass {
    fn name(&self) -> &'static str {
        "osm_overpass"
    }
    fn description(&self) -> &'static str {
        "Run an Overpass QL query against the OSM Overpass API (keyless) and summarize the \
        returned elements (id, type, name, lat/lon). The model writes the QL; always include \
        `[out:json]` and a tight bounding box. Use for \"find all X within bbox\" lookups."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<OverpassArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<OverpassArgs>()?;
            let max = args.max.unwrap_or(50).clamp(1, 500) as usize;
            let q = args.query.trim();
            if q.is_empty() {
                return Err(invalid("empty query"));
            }
            let cache = format!("osm_overpass|{max}|{}", crate::constellation::hash_key(q));
            if let Some(c) = server.retrieval_get(&cache).await {
                return Ok(text_result(c));
            }
            let v: Value = send_json_ctx(
                server
                    .http
                    .post("https://overpass-api.de/api/interpreter")
                    .body(format!("data={}", url_encode(q)))
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .header("Accept", "application/json")
                    .header("User-Agent", crate::skills::grid::OVERPASS_UA),
                "overpass",
            )
            .await?;
            let empty = Vec::new();
            let elements = v
                .get("elements")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            if elements.is_empty() {
                return Ok(text_result("Overpass returned 0 elements.".to_string()));
            }
            let mut out = format!(
                "{} element(s) (showing {}):\n",
                elements.len(),
                elements.len().min(max)
            );
            for el in elements.iter().take(max) {
                let typ = el.get("type").and_then(|x| x.as_str()).unwrap_or("?");
                let id = el.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                let lat = el.get("lat").and_then(|x| x.as_f64());
                let lon = el.get("lon").and_then(|x| x.as_f64());
                let name = el
                    .get("tags")
                    .and_then(|t| t.get("name"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let coords = match (lat, lon) {
                    (Some(a), Some(b)) => format!(" ({a:.5}, {b:.5})"),
                    _ => String::new(),
                };
                out.push_str(&format!("  {typ}/{id}{coords}  {name}\n"));
            }
            server.retrieval_put(cache, &out);
            Ok(text_result(out))
        })
    }
}

// ----- osm_elevation -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ElevationArgs {
    /// Pairs of `[lat, lon]` to look up (max 100 per call).
    points: Vec<(f64, f64)>,
}

pub struct OsmElevation;
impl Skill for OsmElevation {
    fn name(&self) -> &'static str {
        "osm_elevation"
    }
    fn description(&self) -> &'static str {
        "Look up ground elevation (meters above sea level) for a batch of lat/lon points via \
        the keyless Open-Elevation API. Up to 100 points per call."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ElevationArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ElevationArgs>()?;
            if args.points.is_empty() {
                return Err(invalid("points must not be empty"));
            }
            if args.points.len() > 100 {
                return Err(invalid("max 100 points per call"));
            }
            let body = serde_json::json!({
                "locations": args.points.iter().map(|(lat, lon)| serde_json::json!({"latitude": lat, "longitude": lon})).collect::<Vec<_>>()
            });
            let v: Value = send_json_ctx(
                server
                    .http
                    .post("https://api.open-elevation.com/api/v1/lookup")
                    .json(&body),
                "open-elevation",
            )
            .await?;
            let empty = Vec::new();
            let results = v
                .get("results")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            let mut out = format!("Elevation for {} point(s):\n", results.len());
            for r in results {
                let lat = r.get("latitude").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let lon = r.get("longitude").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let el = r.get("elevation").and_then(|x| x.as_f64()).unwrap_or(0.0);
                out.push_str(&format!("  ({lat:>9.5}, {lon:>10.5})  {el:>7.1} m\n"));
            }
            Ok(text_result(out))
        })
    }
}

// ----- osm_route -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RouteArgs {
    /// Origin lat/lon.
    from_lat: f64,
    from_lon: f64,
    /// Destination lat/lon.
    to_lat: f64,
    to_lon: f64,
    /// Profile: "driving" (default), "walking", or "cycling".
    #[serde(default)]
    profile: Option<String>,
}

pub struct OsmRoute;
impl Skill for OsmRoute {
    fn name(&self) -> &'static str {
        "osm_route"
    }
    fn description(&self) -> &'static str {
        "Compute a driving/walking/cycling route between two points via the OSRM public demo \
        server (keyless). Returns distance (m + km), duration (s + min), and a turn summary. \
        For pure great-circle distance use `geo_distance`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RouteArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<RouteArgs>()?;
            let profile = args
                .profile
                .as_deref()
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "driving".into());
            if !matches!(profile.as_str(), "driving" | "walking" | "cycling") {
                return Err(invalid("profile must be driving, walking, or cycling"));
            }
            let url = format!(
                "https://router.project-osrm.org/route/v1/{profile}/{},{};{},{}?overview=false&steps=false",
                args.from_lon, args.from_lat, args.to_lon, args.to_lat
            );
            let v = http_json(server, &url).await?;
            let routes = v.get("routes").and_then(|x| x.as_array());
            let Some(first) = routes.and_then(|r| r.first()) else {
                return Ok(text_result("No route found.".to_string()));
            };
            let dist = first
                .get("distance")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let dur = first
                .get("duration")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let out = format!(
                "{} route: ({:.5}, {:.5}) → ({:.5}, {:.5})\n  distance: {:.0} m  ({:.2} km)\n  duration: {:.0} s  ({:.1} min)",
                profile,
                args.from_lat,
                args.from_lon,
                args.to_lat,
                args.to_lon,
                dist,
                dist / 1000.0,
                dur,
                dur / 60.0,
            );
            Ok(text_result(out))
        })
    }
}

// ----- gis_bbox -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BboxArgs {
    /// List of `[lat, lon]` pairs.
    points: Vec<(f64, f64)>,
}

pub struct GisBbox;
impl Skill for GisBbox {
    fn name(&self) -> &'static str {
        "gis_bbox"
    }
    fn description(&self) -> &'static str {
        "Compute the minimum bounding box (min/max lat & lon) of a list of points. Useful for \
        scoping an Overpass query."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BboxArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<BboxArgs>()?;
            if args.points.is_empty() {
                return Err(invalid("points must not be empty"));
            }
            let (mut min_lat, mut max_lat) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut min_lon, mut max_lon) = (f64::INFINITY, f64::NEG_INFINITY);
            for (lat, lon) in &args.points {
                min_lat = min_lat.min(*lat);
                max_lat = max_lat.max(*lat);
                min_lon = min_lon.min(*lon);
                max_lon = max_lon.max(*lon);
            }
            Ok(text_result(format!(
                "BBox of {} points:\n  south {:.6}  north {:.6}\n  west  {:.6}  east  {:.6}\n  Overpass bbox: ({:.6},{:.6},{:.6},{:.6})",
                args.points.len(),
                min_lat,
                max_lat,
                min_lon,
                max_lon,
                min_lat,
                min_lon,
                max_lat,
                max_lon,
            )))
        })
    }
}

// ----- gis_point_in_polygon -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PointInPolyArgs {
    /// Point to test as `[lat, lon]`.
    point: (f64, f64),
    /// Polygon as a list of `[lat, lon]` vertices in order. Closed automatically.
    polygon: Vec<(f64, f64)>,
}

/// Standard ray-casting point-in-polygon test (treats lat/lon as planar — fine
/// for small regions, OK approximation for state-sized).
fn point_in_polygon(p: (f64, f64), poly: &[(f64, f64)]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let (px, py) = (p.1, p.0); // x = lon, y = lat
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (poly[i].1, poly[i].0);
        let (xj, yj) = (poly[j].1, poly[j].0);
        if ((yi > py) != (yj > py)) && (px < (xj - xi) * (py - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub struct GisPointInPolygon;
impl Skill for GisPointInPolygon {
    fn name(&self) -> &'static str {
        "gis_point_in_polygon"
    }
    fn description(&self) -> &'static str {
        "Test whether a lat/lon point falls inside a polygon (planar ray-casting; fine for small \
        regions). Polygon is closed automatically — pass vertices in order. Returns inside/outside."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PointInPolyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<PointInPolyArgs>()?;
            if args.polygon.len() < 3 {
                return Err(invalid("polygon needs at least 3 vertices"));
            }
            let inside = point_in_polygon(args.point, &args.polygon);
            Ok(text_result(format!(
                "Point ({:.6}, {:.6}) is {} the polygon ({} vertices).",
                args.point.0,
                args.point.1,
                if inside { "INSIDE" } else { "OUTSIDE" },
                args.polygon.len()
            )))
        })
    }
}

// ----- gis_geojson_summary -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GeojsonArgs {
    /// GeoJSON document as a JSON value.
    geojson: Value,
}

pub struct GisGeojsonSummary;
impl Skill for GisGeojsonSummary {
    fn name(&self) -> &'static str {
        "gis_geojson_summary"
    }
    fn description(&self) -> &'static str {
        "Summarize a GeoJSON document: feature count, geometry-type breakdown, and overall \
        bounding box. Accepts a Feature, FeatureCollection, or bare Geometry."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GeojsonArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<GeojsonArgs>()?;
            let mut by_type: std::collections::HashMap<String, u32> =
                std::collections::HashMap::new();
            let mut feature_count = 0u32;
            let (mut min_lat, mut max_lat) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut min_lon, mut max_lon) = (f64::INFINITY, f64::NEG_INFINITY);
            let mut add_coord = |x: f64, y: f64| {
                min_lon = min_lon.min(x);
                max_lon = max_lon.max(x);
                min_lat = min_lat.min(y);
                max_lat = max_lat.max(y);
            };
            fn walk_coords(v: &Value, on: &mut impl FnMut(f64, f64)) {
                if let Value::Array(a) = v {
                    if a.len() == 2 || a.len() == 3 {
                        if let (Some(x), Some(y)) = (a[0].as_f64(), a[1].as_f64()) {
                            on(x, y);
                            return;
                        }
                    }
                    for el in a {
                        walk_coords(el, on);
                    }
                }
            }
            fn process_geometry(
                g: &Value,
                by_type: &mut std::collections::HashMap<String, u32>,
                add_coord: &mut impl FnMut(f64, f64),
            ) {
                if let Some(t) = g.get("type").and_then(|x| x.as_str()) {
                    *by_type.entry(t.to_string()).or_insert(0) += 1;
                }
                if let Some(c) = g.get("coordinates") {
                    walk_coords(c, add_coord);
                }
            }
            let top_type = args
                .geojson
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            match top_type {
                "FeatureCollection" => {
                    if let Some(features) = args.geojson.get("features").and_then(|x| x.as_array())
                    {
                        feature_count = features.len() as u32;
                        for f in features {
                            if let Some(g) = f.get("geometry") {
                                process_geometry(g, &mut by_type, &mut add_coord);
                            }
                        }
                    }
                }
                "Feature" => {
                    feature_count = 1;
                    if let Some(g) = args.geojson.get("geometry") {
                        process_geometry(g, &mut by_type, &mut add_coord);
                    }
                }
                _ => {
                    process_geometry(&args.geojson, &mut by_type, &mut add_coord);
                }
            }
            let mut out = format!(
                "GeoJSON top-level type: {top_type}\n  features: {feature_count}\n  geometries by type:\n"
            );
            let mut types: Vec<(String, u32)> = by_type.into_iter().collect();
            types.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            for (t, n) in &types {
                out.push_str(&format!("    {t:<18}  {n}\n"));
            }
            if min_lat.is_finite() {
                out.push_str(&format!(
                    "  bbox: south {min_lat:.6}, north {max_lat:.6}, west {min_lon:.6}, east {max_lon:.6}\n"
                ));
            }
            Ok(text_result(out))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(OsmGeocode),
        Box::new(OsmReverseGeocode),
        Box::new(OsmOverpass),
        Box::new(OsmElevation),
        Box::new(OsmRoute),
        Box::new(GisBbox),
        Box::new(GisPointInPolygon),
        Box::new(GisGeojsonSummary),
    ]
}

#[cfg(test)]
mod live {
    use super::*;

    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// Nominatim — keyless geocode. The OSM UA policy requires a descriptive
    /// User-Agent (same lesson as Overpass), so the http() builder uses it.
    #[tokio::test]
    #[ignore]
    async fn nominatim_geocode_live() {
        let r = http()
            .get("https://nominatim.openstreetmap.org/search?q=Redmond%2C+WA&format=json&limit=1")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let arr = v.as_array().expect("expected JSON array");
        assert!(!arr.is_empty());
        for k in ["lat", "lon", "display_name", "class", "type"] {
            assert!(arr[0].get(k).is_some(), "missing field {k}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn nominatim_reverse_live() {
        let r = http()
            .get("https://nominatim.openstreetmap.org/reverse?lat=47.6700&lon=-122.1200&format=json&zoom=18")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        assert!(v.get("display_name").is_some());
        assert!(v["address"].is_object());
    }

    /// Overpass: the bug from the previous fix. The osm_overpass skill code
    /// path sends the same UA + Accept; this test catches a regression there.
    #[tokio::test]
    #[ignore]
    async fn osm_overpass_post_live() {
        // Single-substation tiny bbox so the call is fast and we never get
        // rate-limited even on a CI nightly.
        let ql = "[out:json][timeout:30];(node[\"power\"=\"substation\"](47.66,-122.13,47.68,-122.11);way[\"power\"=\"substation\"](47.66,-122.13,47.68,-122.11);relation[\"power\"=\"substation\"](47.66,-122.13,47.68,-122.11););out center tags;";
        let body = format!("data={}", url_encode(ql));
        let r = http()
            .post("https://overpass-api.de/api/interpreter")
            .body(body)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        assert_eq!(v["version"].as_f64(), Some(0.6));
    }

    #[tokio::test]
    #[ignore]
    async fn open_elevation_live() {
        let body = serde_json::json!({"locations": [{"latitude": 47.67, "longitude": -122.12}]});
        let r = http()
            .post("https://api.open-elevation.com/api/v1/lookup")
            .json(&body)
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let results = v["results"].as_array().expect("missing results");
        assert!(!results.is_empty());
        assert!(results[0].get("elevation").is_some());
    }

    /// OSRM public demo — very short route to keep the call cheap.
    #[tokio::test]
    #[ignore]
    async fn osrm_route_live() {
        let r = http()
            .get("https://router.project-osrm.org/route/v1/driving/-122.12,47.67;-122.11,47.68?overview=false&steps=false")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        assert_eq!(v["code"].as_str(), Some("Ok"));
        let routes = v["routes"].as_array().expect("missing routes");
        assert!(!routes.is_empty());
        for k in ["distance", "duration"] {
            assert!(routes[0].get(k).is_some(), "missing field {k}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_in_polygon_basic() {
        // Unit square (lat=y, lon=x).
        let poly = vec![(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)];
        assert!(point_in_polygon((0.5, 0.5), &poly));
        assert!(!point_in_polygon((1.5, 0.5), &poly));
        assert!(!point_in_polygon((-0.5, -0.5), &poly));
    }
}
