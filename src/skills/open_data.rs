//! Open data feeds — keyless HTTP fetches for OpenSky aircraft state,
//! USGS Earthquake live GeoJSON, NOAA SWPC solar wind, GeoNames, GBIF.
//!
//! ## Constellation sharing
//!
//! Every response is keyed by its query parameters and goes through
//! `retrieval_get` / `retrieval_put` so a constellation peer that already
//! fetched the same window (`opensky|<bbox>`, `usgs_quake|<min>|<period>`,
//! `swpc|plasma-1-day`) can serve it within the cache TTL. Live feeds are
//! still live — the cache TTL governs how stale a peer-served response may
//! be — but the upstream isn't hammered by every node independently.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, text_result};

async fn fetch(server: &crate::Lodestone, url: &str) -> std::result::Result<String, McpError> {
    let r = server
        .http
        .get(url)
        .send()
        .await
        .map_err(|e| internal(anyhow::anyhow!(e)))?;
    if !r.status().is_success() {
        return Err(internal(anyhow::anyhow!(
            "{} returned status {}",
            url,
            r.status()
        )));
    }
    let body = r.text().await.map_err(|e| internal(anyhow::anyhow!(e)))?;
    Ok(truncate_chars(&body, server.max_chars))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OpenSkyArgs {
    /// Bounding box: [lat_min, lon_min, lat_max, lon_max].
    bbox: Option<[f64; 4]>,
}

pub struct OpenSkyStates;
impl Skill for OpenSkyStates {
    fn name(&self) -> &'static str {
        "opensky_states"
    }
    fn description(&self) -> &'static str {
        "Live ADS-B aircraft state vectors from OpenSky Network's keyless \
        REST endpoint. Optional bbox = [lat_min, lon_min, lat_max, lon_max] \
        narrows the query. Returns the raw OpenSky JSON (`time`, `states[]` \
        — each row is a 17-element tuple per OpenSky spec)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<OpenSkyArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            let (_s, a) = ctx.parse::<OpenSkyArgs>()?;
            let (url, key) = match a.bbox {
                Some(b) => (
                    format!(
                        "https://opensky-network.org/api/states/all?lamin={}&lomin={}&lamax={}&lomax={}",
                        b[0], b[1], b[2], b[3]
                    ),
                    format!("opensky|{},{},{},{}", b[0], b[1], b[2], b[3]),
                ),
                None => (
                    "https://opensky-network.org/api/states/all".into(),
                    "opensky|all".into(),
                ),
            };
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let body = fetch(server, &url).await?;
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Aircraft over Switzerland",
                args: r#"{"bbox": [45.8, 5.9, 47.8, 10.5]}"#,
                note: Some("Bbox order: [lat_min, lon_min, lat_max, lon_max]."),
            },
            SkillExample {
                title: "Global state vectors",
                args: r#"{}"#,
                note: Some("Omit `bbox` for the world feed; payload is large."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Snapshot live ADS-B traffic in a geographic window.",
            "Pull the worldwide aircraft state for downstream filtering.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EarthquakeArgs {
    /// `hour`, `day`, `week`, or `month`.
    #[serde(default)]
    period: Option<String>,
    /// Minimum magnitude: `all`, `1.0`, `2.5`, `4.5`, `significant`.
    #[serde(default)]
    minimum: Option<String>,
}

pub struct UsgsEarthquakes;
impl Skill for UsgsEarthquakes {
    fn name(&self) -> &'static str {
        "usgs_earthquakes"
    }
    fn description(&self) -> &'static str {
        "Real-time earthquake feed from USGS, returned as GeoJSON. Combine \
        `period` (hour/day/week/month) and `minimum` (all / 1.0 / 2.5 / \
        4.5 / significant). Each feature has properties.mag, place, time, \
        url, …; geometry is [lon, lat, depth_km]."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EarthquakeArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            let (_s, a) = ctx.parse::<EarthquakeArgs>()?;
            let period = a.period.unwrap_or_else(|| "day".into());
            let minimum = a.minimum.unwrap_or_else(|| "2.5".into());
            let key = format!("usgs_quake|{minimum}|{period}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!(
                "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/{minimum}_{period}.geojson"
            );
            let body = fetch(server, &url).await?;
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Significant quakes this week",
                args: r#"{"period": "week", "minimum": "significant"}"#,
                note: Some("Returns USGS GeoJSON FeatureCollection."),
            },
            SkillExample {
                title: "M4.5+ in the last day",
                args: r#"{"period": "day", "minimum": "4.5"}"#,
                note: None,
            },
            SkillExample {
                title: "Everything in the last hour",
                args: r#"{"period": "hour", "minimum": "all"}"#,
                note: Some("`all` includes tiny background quakes; can be noisy."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pull the real-time USGS quake feed for a magnitude/timeframe.",
            "Drive a dashboard or alert pipeline off significant seismic events.",
            "Get raw GeoJSON for downstream mapping.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::OneOf {
                field: "period",
                values: &["hour", "day", "week", "month"],
            },
            Rule::OneOf {
                field: "minimum",
                values: &["all", "1.0", "2.5", "4.5", "significant"],
            },
        ]
    }
}

pub struct SwpcSolarWind;
impl Skill for SwpcSolarWind {
    fn name(&self) -> &'static str {
        "swpc_solar_wind"
    }
    fn description(&self) -> &'static str {
        "Real-time solar wind (DSCOVR / ACE plasma + magnetic field) from \
        NOAA SWPC. Returns the JSON arrays the SWPC dashboard ingests."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            let key = "swpc|plasma-1-day".to_string();
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let body = fetch(
                server,
                "https://services.swpc.noaa.gov/products/solar-wind/plasma-1-day.json",
            )
            .await?;
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Pull the last 24 hours of plasma data",
            args: r#"{}"#,
            note: Some("Takes no arguments; returns SWPC plasma JSON arrays."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check current solar-wind density, speed, and temperature near L1.",
            "Feed a geomagnetic-storm watcher with raw DSCOVR/ACE plasma data.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(OpenSkyStates),
        Box::new(UsgsEarthquakes),
        Box::new(SwpcSolarWind),
    ]
}
