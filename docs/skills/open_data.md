# Open data feeds — `open_data_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/open_data.rs`](../../src/skills/open_data.rs) |
| **Tools** | `open_data_opensky_states`, `open_data_usgs_earthquakes`, `open_data_swpc_solar_wind` |
| **Network** | yes — three keyless live feeds |
| **Default** | on; gateable via `[tools]` |

## What it does

Pulls from three keyless live public feeds — aircraft state vectors
(OpenSky), USGS earthquake GeoJSON, and NOAA SWPC real-time solar wind.
Pure read-through; the server doesn't cache the responses beyond the
standard provider/result cache. Each upstream's polite-use policy
applies (don't poll faster than they say).

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `open_data_opensky_states` | `bbox?` (`[min_lat, min_lon, max_lat, max_lon]`) | OpenSky `/api/states/all`; aircraft state vectors (ICAO24, callsign, lat/lon, alt, vel, …). |
| `open_data_usgs_earthquakes` | `minimum?` (`1.0`/`2.5`/`4.5`/`significant`), `period?` (`hour`/`day`/`week`/`month`) | USGS GeoJSON earthquake feed. |
| `open_data_swpc_solar_wind` | — | NOAA SWPC `plasma-1-day.json` — Bz, density, speed. |

## Example uses

- **Air traffic near a point.** `open_data_opensky_states { bbox:
  [40.5, -74.5, 41.0, -73.5] }` for the NYC area → a list of aircraft.
- **Recent earthquakes.** `open_data_usgs_earthquakes { minimum: "2.5",
  period: "day" }` → the M≥2.5 events in the last 24 h.
- **Solar context.** `open_data_swpc_solar_wind` — feed the speed +
  density into a back-of-envelope geomagnetic-storm risk check (and
  pair with `atm_space_weather_kp`).

## Notes

- **Reliability is not guaranteed.** Public feeds rate-limit and rotate
  endpoints. If a feed is down, the tool returns a clean error; falling
  back to a cached or alternate source is the caller's job.
- **OpenSky** anonymizes some fields and rate-limits anonymous queries.

## See also

- [tools.md](../tools.md)
- [skills/atmospheric.md](atmospheric.md) — `atm_space_weather_kp` from
  the same SWPC family.
- [skills/noaa.md](noaa.md) — US NWS alerts / forecasts.
