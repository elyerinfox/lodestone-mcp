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
use crate::util::url_enc;
use crate::{invalid, text_result};

// Not gated by config — keyless public APIs, always on (like wikipedia/arxiv).

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
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Overpass,
        }
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
                url_enc(args.query.trim())
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "City and state",
                args: r#"{"query": "Redmond, WA"}"#,
                note: Some("Returns up to 5 ranked candidates with lat/lon and OSM ids."),
            },
            SkillExample {
                title: "Landmark",
                args: r#"{"query": "Eiffel Tower"}"#,
                note: None,
            },
            SkillExample {
                title: "Cap results",
                args: r#"{"query": "Springfield", "max": 10}"#,
                note: Some("`max` is clamped to 1..=25."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Turn a free-form place name or address into lat/lon.",
            "Disambiguate a place by inspecting ranked candidates with admin context.",
            "Get an OSM id to feed into a follow-up Overpass or routing call.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "query",
            min: Some(1),
            max: None,
        }]
    }
}

// ----- osm_reverse_geocode -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReverseArgs {
    /// Latitude in decimal degrees.
    lat: f64,
    /// Longitude in decimal degrees.
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
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Overpass,
        }
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Building-level address",
                args: r#"{"lat": 47.6700, "lon": -122.1200}"#,
                note: Some("Defaults to `zoom: 18` (building-level)."),
            },
            SkillExample {
                title: "City-level lookup",
                args: r#"{"lat": 48.8584, "lon": 2.2945, "zoom": 10}"#,
                note: Some("Lower `zoom` resolves to a coarser admin unit."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Find the street address or place name at a known coordinate.",
            "Pull admin context (city, state, country) for a lat/lon.",
            "Label a sub-point produced by routing or satellite sub-point math.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
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
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Overpass,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<OverpassArgs>()?;
            let max = args.max.unwrap_or(50).clamp(1, 500) as usize;
            let q = args.query.trim();
            if q.is_empty() {
                return Err(invalid("empty query"));
            }
            // The QL hash is the cross-skill identifier: `grid_*` tools and
            // `osm_overpass` running the same query share the same source-id
            // even though their primary keys differ.
            let q_hash = crate::constellation::hash_key(q);
            let cache = format!("osm_overpass|{max}|{q_hash}");
            let ids = crate::constellation::Identifiers::new(&cache)
                .with_source(crate::constellation::Source::Overpass)
                .with_source_id("overpass_qhash", &q_hash);
            if let Some(c) = server.retrieval_lookup(&ids).await {
                return Ok(text_result(c));
            }
            let v: Value = send_json_ctx(
                server
                    .http
                    .post("https://overpass-api.de/api/interpreter")
                    .body(format!("data={}", url_enc(q)))
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
            server.retrieval_put_indexed(&ids, &out);
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Pharmacies in a small bbox",
                args: r#"{"query": "[out:json][timeout:25];node[\"amenity\"=\"pharmacy\"](47.6,-122.2,47.7,-122.1);out;"}"#,
                note: Some(
                    "Always include `[out:json]` and a tight bbox; default `max` is 50 elements.",
                ),
            },
            SkillExample {
                title: "Cafes in a Berlin bbox, more results",
                args: r#"{"query": "[out:json][timeout:25];node[\"amenity\"=\"cafe\"](52.50,13.38,52.53,13.42);out;", "max": 200}"#,
                note: Some("`max` is capped at 500."),
            },
            SkillExample {
                title: "Hospital ways/relations with name",
                args: r#"{"query": "[out:json][timeout:25];(way[\"amenity\"=\"hospital\"](51.50,-0.20,51.55,-0.10);relation[\"amenity\"=\"hospital\"](51.50,-0.20,51.55,-0.10););out center;"}"#,
                note: Some(
                    "Use `out center;` for non-node features to get a representative lat/lon.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Find all features of a given tag inside a bounding box.",
            "Enumerate amenities / POIs in a small map region.",
            "Pull OSM ids + names for downstream geocoding or rendering.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "query",
            min: Some(1),
            max: None,
        }]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Single point",
                args: r#"{"points": [[47.67, -122.12]]}"#,
                note: Some("Returns ground elevation in meters above sea level."),
            },
            SkillExample {
                title: "Batch lookup",
                args: r#"{"points": [[27.9881, 86.9250], [35.3606, 138.7274], [44.4280, -110.5885]]}"#,
                note: Some("Up to 100 points per call."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Get terrain elevation for one or many lat/lon points.",
            "Sample a profile along a route by spacing points beforehand.",
            "Estimate antenna height-above-terrain for line-of-sight reasoning.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "points",
            min: Some(1),
            max: Some(100),
        }]
    }
}

// ----- osm_route -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RouteArgs {
    /// Origin lat/lon.
    from_lat: f64,
    /// Origin longitude.
    from_lon: f64,
    /// Destination lat/lon.
    to_lat: f64,
    /// Destination longitude.
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Driving route",
                args: r#"{"from_lat": 47.67, "from_lon": -122.12, "to_lat": 47.61, "to_lon": -122.33}"#,
                note: Some("`profile` defaults to driving."),
            },
            SkillExample {
                title: "Walking route",
                args: r#"{"from_lat": 51.5007, "from_lon": -0.1246, "to_lat": 51.5074, "to_lon": -0.0901, "profile": "walking"}"#,
                note: None,
            },
            SkillExample {
                title: "Cycling route",
                args: r#"{"from_lat": 52.5200, "from_lon": 13.4050, "to_lat": 52.5163, "to_lon": 13.3777, "profile": "cycling"}"#,
                note: Some("For pure great-circle distance use `geo_distance` instead."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate real road/walk/bike distance and travel time between two points.",
            "Compare driving vs walking vs cycling time for the same OD pair.",
            "Sanity-check a great-circle distance against an actual routable path.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "from_lat",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "from_lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "to_lat",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "to_lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Two points",
                args: r#"{"points": [[47.5, -122.3], [47.7, -122.1]]}"#,
                note: Some(
                    "Output includes a ready-to-paste Overpass `(south,west,north,east)` bbox.",
                ),
            },
            SkillExample {
                title: "Polygon vertices",
                args: r#"{"points": [[40.70, -74.02], [40.78, -73.96], [40.74, -73.91], [40.71, -74.00]]}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Derive a tight bbox from a cluster of POIs to scope an Overpass query.",
            "Compute the extent of a route or trajectory for map framing.",
            "Bound a set of geocoded results before a `grid_*` infrastructure call.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "points",
            min: Some(1),
            max: None,
        }]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Inside a square",
                args: r#"{"point": [0.5, 0.5], "polygon": [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]}"#,
                note: Some("Returns `INSIDE` or `OUTSIDE` plus the vertex count."),
            },
            SkillExample {
                title: "Real-world polygon test",
                args: r#"{"point": [47.6062, -122.3321], "polygon": [[47.50, -122.45], [47.50, -122.20], [47.75, -122.20], [47.75, -122.45]]}"#,
                note: Some("Planar test — fine up to state-sized regions."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check whether a coordinate falls inside a hand-drawn region.",
            "Filter a list of points to those within a named admin polygon.",
            "Decide a binary in/out flag without firing a spatial DB query.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "polygon",
            min: Some(3),
            max: None,
        }]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Single Point feature",
                args: r#"{"geojson": {"type": "Feature", "geometry": {"type": "Point", "coordinates": [-122.12, 47.67]}, "properties": {}}}"#,
                note: Some("Reports 1 feature, a Point in `geometries by type`, and the bbox."),
            },
            SkillExample {
                title: "FeatureCollection",
                args: r#"{"geojson": {"type": "FeatureCollection", "features": [{"type": "Feature", "geometry": {"type": "Point", "coordinates": [0.0, 0.0]}, "properties": {}}, {"type": "Feature", "geometry": {"type": "LineString", "coordinates": [[0.0, 0.0], [1.0, 1.0]]}, "properties": {}}]}}"#,
                note: None,
            },
            SkillExample {
                title: "Bare geometry",
                args: r#"{"geojson": {"type": "Polygon", "coordinates": [[[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]]}}"#,
                note: Some("Accepts Feature, FeatureCollection, or a bare Geometry."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Get feature count and geometry-type breakdown of an unknown GeoJSON blob.",
            "Pull the overall bbox of a FeatureCollection without parsing it yourself.",
            "Quickly sanity-check a GeoJSON file before downstream processing.",
        ]
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
        let body = format!("data={}", url_enc(ql));
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
