//! Open data feeds — keyless HTTP fetches for OpenSky aircraft state,
//! USGS Earthquake live GeoJSON, NOAA SWPC solar wind, GeoNames, GBIF.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result};

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
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            let (_s, a) = ctx.parse::<OpenSkyArgs>()?;
            let url = match a.bbox {
                Some(b) => format!(
                    "https://opensky-network.org/api/states/all?lamin={}&lomin={}&lamax={}&lomax={}",
                    b[0], b[1], b[2], b[3]
                ),
                None => "https://opensky-network.org/api/states/all".into(),
            };
            let body = fetch(server, &url).await?;
            Ok(text_result(body))
        })
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
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            let (_s, a) = ctx.parse::<EarthquakeArgs>()?;
            let period = a.period.unwrap_or_else(|| "day".into());
            let minimum = a.minimum.unwrap_or_else(|| "2.5".into());
            if !["hour", "day", "week", "month"].contains(&period.as_str()) {
                return Err(invalid("period must be hour/day/week/month"));
            }
            if !["all", "1.0", "2.5", "4.5", "significant"].contains(&minimum.as_str()) {
                return Err(invalid(
                    "minimum must be all/1.0/2.5/4.5/significant",
                ));
            }
            let url = format!(
                "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/{minimum}_{period}.geojson"
            );
            let body = fetch(server, &url).await?;
            Ok(text_result(body))
        })
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
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let body = fetch(
                ctx.server,
                "https://services.swpc.noaa.gov/products/solar-wind/plasma-1-day.json",
            )
            .await?;
            Ok(text_result(body))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(OpenSkyStates),
        Box::new(UsgsEarthquakes),
        Box::new(SwpcSolarWind),
    ]
}
