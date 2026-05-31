# Weather forecasts — `weather_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/weather.rs`](../../src/skills/weather.rs) |
| **Tools** | `weather_forecast`, `weather_marine`, `weather_air_quality`, `weather_historical` |
| **Network** | Open-Meteo (`api.open-meteo.com`) — **keyless** |
| **Default** | **on** |
| **Config** | none — always-on like Wikipedia / arxiv |

## What it does

Point weather queries against **Open-Meteo**, which aggregates the same
underlying numerical-weather-prediction (NWP) models Ventusky visualizes
(GFS, GEM, ICON, ECMWF IFS04 / 025, JMA, MetOffice, MET Norway, ARPEGE, …),
plus the **ERA5** reanalysis archive (1940 → ~5 days ago), marine forecasts,
and air quality.

For visual layers Ventusky overlays — radar composites (EURAD / USRAD /
WORAD) and live satellite imagery — those are PNG tile services, not point
APIs; pull them via `fetch_page` / `webpage_to_pdf` if you need an image.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `weather_forecast` | `lat`, `lon`, `hourly?`, `daily?`, `model?`, `forecast_days?`, `hours?`, `timezone?` | Point forecast. `model`: `best_match` (default) / `gfs_seamless` / `ecmwf_ifs04` / `ecmwf_ifs025` / `icon_seamless` / `gem_seamless` / `jma_seamless` / `metno_seamless` / `ukmo_seamless` / `arpege_seamless`. |
| `weather_marine` | `lat`, `lon`, `hourly?`, `days?`, `timezone?` | Marine: wave height / period / direction, swell, SST. |
| `weather_air_quality` | `lat`, `lon`, `hourly?`, `days?`, `timezone?` | Air quality: PM10 / PM2.5 / NO₂ / O₃ / CO / SO₂, European AQI, dust, pollen. |
| `weather_historical` | `lat`, `lon`, `start_date`, `end_date`, `hourly?`, `daily?`, `timezone?` | ERA5 reanalysis archive (1940-01-01 → ~5 days ago). |

The `hourly` / `daily` args are comma-separated Open-Meteo variable lists. A
sensible surface default is used when both are omitted.

## Example uses

- **Tomorrow's forecast** —
  `weather_forecast { lat: 47.67, lon: -122.12, forecast_days: 2 }`.
- **Compare models** — call with `model: "ecmwf_ifs04"`, then `gfs_seamless`,
  see how they diverge.
- **Wave height** —
  `weather_marine { lat: 47.67, lon: -125.0 }`.
- **History** —
  `weather_historical { lat: 47.67, lon: -122.12, start_date: "2024-01-01", end_date: "2024-12-31", daily: "temperature_2m_max,temperature_2m_min" }`.

## Notes

- **Keyless.** Open-Meteo asks for reasonable use; cached aggressively.
- **Times.** All timestamps are ISO-8601; `timezone` accepts IANA names
  (`America/Los_Angeles`) or `"auto"` for local-to-lat/lon.

## See also

- [tools.md](../tools.md)
- [skills/noaa.md](noaa.md) — NWS alerts and US-specific forecasts.
