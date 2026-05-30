//! NASA open-data skills (keyless-friendly): the api.nasa.gov endpoints. Works with
//! no key via `DEMO_KEY` (low rate limit); an optional free `[nasa].key` raises it.
//! Results are cached. `nasa_neo` (near-Earth objects for a day),
//! `nasa_mars_photos` (rover imagery).
//!
//! ⚠ **`nasa_mars_photos` is currently broken upstream.** The Mars Rover Photos
//! API was hosted on a Heroku free dyno that NASA never migrated; the endpoint
//! `api.nasa.gov/mars-photos/...` now returns 404 ("No such app"). The skill
//! still ships so the contract surface stays stable, but every call will
//! surface NASA's 404 verbatim. The `nasa_mars_photos_currently_down`
//! integration test asserts this state so we'll notice if NASA restores it.

use std::sync::Arc;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, text_result, Lodestone};

/// The effective API key: the configured one, or NASA's public `DEMO_KEY`.
fn api_key(server: &Lodestone) -> String {
    let k = server.nasa_key.trim();
    if k.is_empty() {
        "DEMO_KEY".to_string()
    } else {
        k.to_string()
    }
}

async fn get_json(http: &Client, url: &str) -> Result<Value> {
    Ok(http
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()
        .context("NASA API request failed (rate limit? DEMO_KEY is very limited — set [nasa].key)")?
        .json()
        .await?)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NeoArgs {
    /// Day as YYYY-MM-DD to list near-Earth objects with close approaches. Omit for today.
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MarsArgs {
    /// Rover name: "curiosity" (default), "perseverance", "opportunity", "spirit".
    #[serde(default)]
    rover: Option<String>,
    /// Martian sol (mission day). Provide this or `earth_date`; defaults to sol 1000.
    #[serde(default)]
    sol: Option<u32>,
    /// Earth date YYYY-MM-DD (alternative to `sol`).
    #[serde(default)]
    earth_date: Option<String>,
    /// Max photos to list. Default 10, capped 25.
    #[serde(default)]
    max_results: Option<u32>,
}

pub struct NasaNeo;
impl Skill for NasaNeo {
    fn name(&self) -> &'static str {
        "nasa_neo"
    }
    fn description(&self) -> &'static str {
        "NASA near-Earth objects with close approaches on a given day (keyless via DEMO_KEY): name, \
        estimated diameter, potentially-hazardous flag, miss distance, and relative velocity."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NeoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<NeoArgs>()?;
            let date = args.date.as_deref().map(str::trim).unwrap_or("");
            let key = format!("nasa_neo|{date}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let mut url = format!(
                "https://api.nasa.gov/neo/rest/v1/feed?api_key={}",
                api_key(server)
            );
            if !date.is_empty() {
                url.push_str(&format!("&start_date={date}&end_date={date}"));
            }
            let v = get_json(&server.http, &url).await.map_err(internal)?;
            let mut out = String::new();
            let mut count = 0usize;
            if let Some(days) = v.get("near_earth_objects").and_then(|x| x.as_object()) {
                for (day, list) in days {
                    let objs = list.as_array().cloned().unwrap_or_default();
                    out.push_str(&format!("Near-Earth objects on {day} ({}):\n", objs.len()));
                    for o in objs.iter().take(40) {
                        count += 1;
                        let name = o.get("name").and_then(|x| x.as_str()).unwrap_or("?");
                        let hazard = o
                            .get("is_potentially_hazardous_asteroid")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false);
                        let dmin = o
                            .pointer("/estimated_diameter/meters/estimated_diameter_min")
                            .and_then(|x| x.as_f64())
                            .unwrap_or(0.0);
                        let dmax = o
                            .pointer("/estimated_diameter/meters/estimated_diameter_max")
                            .and_then(|x| x.as_f64())
                            .unwrap_or(0.0);
                        let ca = o
                            .get("close_approach_data")
                            .and_then(|x| x.as_array())
                            .and_then(|a| a.first());
                        let miss_km = ca
                            .and_then(|c| c.pointer("/miss_distance/kilometers"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("?");
                        let vel = ca
                            .and_then(|c| c.pointer("/relative_velocity/kilometers_per_hour"))
                            .and_then(|x| x.as_str())
                            .unwrap_or("?");
                        out.push_str(&format!(
                            "\n  {name}{}\n    ~{dmin:.0}–{dmax:.0} m · miss {miss_km} km · {vel} km/h\n",
                            if hazard { "  ⚠ hazardous" } else { "" }
                        ));
                    }
                }
            }
            if count == 0 {
                out = "No near-Earth objects reported for that day.".to_string();
            }
            let out = truncate_chars(&out, server.max_chars);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct NasaMarsPhotos;
impl Skill for NasaMarsPhotos {
    fn name(&self) -> &'static str {
        "nasa_mars_photos"
    }
    fn description(&self) -> &'static str {
        "NASA Mars rover photos (keyless via DEMO_KEY): image URLs (with camera + earth date) for a \
        rover on a given sol or earth_date. Use read_pdf/fetch_page only on text; these are images."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MarsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MarsArgs>()?;
            let rover = args
                .rover
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("curiosity")
                .to_ascii_lowercase();
            let limit = crate::clamp(args.max_results, 10, 25);
            let when = match (args.sol, args.earth_date.as_deref()) {
                (_, Some(d)) if !d.trim().is_empty() => format!("earth_date={}", d.trim()),
                (Some(s), _) => format!("sol={s}"),
                _ => "sol=1000".to_string(),
            };
            let key = format!("nasa_mars|{rover}|{when}|{limit}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!(
                "https://api.nasa.gov/mars-photos/api/v1/rovers/{rover}/photos?{when}&api_key={}",
                api_key(server)
            );
            let v = get_json(&server.http, &url).await.map_err(internal)?;
            let photos = v
                .get("photos")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            if photos.is_empty() {
                return Ok(text_result(format!("No {rover} photos for {when}.")));
            }
            let mut out = format!("{} {rover} photo(s) ({when}):\n", photos.len());
            for p in photos.iter().take(limit) {
                let img = p.get("img_src").and_then(|x| x.as_str()).unwrap_or("");
                let cam = p
                    .pointer("/camera/full_name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let date = p.get("earth_date").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!("\n  {img}\n    {cam} · {date}\n"));
            }
            let out = truncate_chars(&out, server.max_chars);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(NasaNeo), Box::new(NasaMarsPhotos)]
}

#[cfg(test)]
mod tests {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)")
            .build()
            .unwrap()
    }

    /// NASA's DEMO_KEY allows ~30 req/IP/hour. Live tests should use a real key
    /// if one is available (LODESTONE_NASA_KEY) and skip cleanly otherwise — so
    /// repeated nightly runs don't fail on the public quota.
    fn nasa_key_or_skip() -> Option<String> {
        match std::env::var("LODESTONE_NASA_KEY").or_else(|_| std::env::var("NASA_API_KEY")) {
            Ok(k) if !k.trim().is_empty() => Some(k),
            _ => {
                eprintln!("skipping nasa live: no LODESTONE_NASA_KEY/NASA_API_KEY (DEMO_KEY rate-limits at 30/hr)");
                None
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn nasa_neo_live() {
        let Some(key) = nasa_key_or_skip() else {
            return;
        };
        let url = format!("https://api.nasa.gov/neo/rest/v1/feed?api_key={key}");
        let r = http()
            .get(&url)
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        assert!(v.get("element_count").is_some());
        assert!(v.get("near_earth_objects").is_some());
    }

    /// As of mid-2025 the Mars Rover Photos API (hosted on Heroku free dynos)
    /// has been DECOMMISSIONED by NASA — `api.nasa.gov/mars-photos/...` 404s
    /// and the underlying app returns "No such app". The `nasa_mars_photos`
    /// skill is therefore non-functional. This test asserts the **outage** so
    /// we'll notice the day NASA restores it (and can then re-enable the
    /// skill / change this test back to assert success).
    #[tokio::test]
    #[ignore]
    async fn nasa_mars_photos_currently_down() {
        let Some(key) = nasa_key_or_skip() else {
            return;
        };
        let url = format!("https://api.nasa.gov/mars-photos/api/v1/rovers/curiosity/photos?sol=1000&api_key={key}");
        let r = http().get(&url).send().await.expect("network");
        // When NASA restores the service the status will flip away from 404
        // — and this test will fail, prompting us to re-enable the skill.
        assert_eq!(
            r.status().as_u16(),
            404,
            "Mars Photos API responded — restore the skill and switch this test to assert success"
        );
    }
}
