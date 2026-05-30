# NASA open data — `nasa_neo` / `nasa_mars_photos`

|  |  |
| --- | --- |
| **Module** | [`src/skills/nasa.rs`](../../src/skills/nasa.rs) |
| **Tools** | `nasa_neo`, `nasa_mars_photos` |
| **Network** | keyless API (api.nasa.gov via `DEMO_KEY`; optional free key) |
| **Default** | always on (no enable gate; `[nasa].key` optional, tools gateable via `[tools]`) |
| **Config** | `[nasa]` in [`config/17-data-apis.toml`](../../config/17-data-apis.toml); `DEMO_KEY` unless a key is set |

## What it does
Two read-only scientific lookups against the public api.nasa.gov endpoints:
the near-Earth-object feed for a day, and Mars rover imagery metadata. They
work **keyless** by sending NASA's shared `DEMO_KEY` (a very low rate limit);
setting a free key raises it. Results are cached, so repeated identical queries
don't re-hit the (limited) endpoint.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `nasa_neo` | `date?` | Near-Earth objects with close approaches on a day: name, estimated diameter (m), potentially-hazardous flag, miss distance (km), relative velocity (km/h). Up to 40 listed. Omit `date` for today. |
| `nasa_mars_photos` | `rover?`, `sol?`, `earth_date?`, `max_results?` | Mars rover photo URLs with camera + earth date. `rover` defaults to `curiosity` (also `perseverance`/`opportunity`/`spirit`); pass `sol` (martian day, default 1000) or `earth_date` (`YYYY-MM-DD`); `max_results` default 10, capped 25. |

## Configuration & gating
- `[nasa].key` (env `LODESTONE_NASA_KEY`, also accepts `NASA_API_KEY`) — empty
  string means `DEMO_KEY`. A key is a credential: prefer the env var, never commit
  a real one. It only raises the rate limit; the tools never require it.
- The whole family is gated by `[nasa]` (on by default) in
  [`config/17-data-apis.toml`](../../config/17-data-apis.toml). Individual tools
  are also gateable via `[tools]`.
- Both tools are read-only (no confirmation guard).

## Example uses
- **What's passing by** — `nasa_neo` for today, scan the hazardous flags and miss
  distances.
- **Recent rover shots** — `nasa_mars_photos rover=perseverance earth_date=2024-01-01`,
  then open the returned image URLs (these are images — don't `read_pdf`/`fetch_page` them).

## See also
[tools.md](../tools.md)
