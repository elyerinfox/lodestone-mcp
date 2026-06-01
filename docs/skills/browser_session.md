# Browser sessions — `browser_open` / `browser_navigate` / `browser_click` / `browser_type` / `browser_wait` / `browser_extract` / `browser_eval` / `browser_screenshot` / `browser_list` / `browser_close` / `browser_persona_get` / `browser_persona_list` / `browser_persona_reset` / `browser_persona_delegate`

|  |  |
| --- | --- |
| **Module** | [`src/skills/browser_session.rs`](../../src/skills/browser_session.rs) |
| **Tools** | `browser_open`, `browser_navigate`, `browser_click`, `browser_type`, `browser_wait`, `browser_extract`, `browser_eval`, `browser_screenshot`, `browser_list`, `browser_close`, `browser_persona_get`, `browser_persona_list`, `browser_persona_reset`, `browser_persona_delegate` |
| **Network** | local Chromium subprocess (CDP) + outbound HTTPS to whatever the model navigates to |
| **Default** | **on** when Chrome/Chromium is on `PATH` — falls back to a clear "browser unavailable" error otherwise |
| **Config** | `[google].chrome_path` for the binary location; `[network.capabilities].browser` to opt in to peer-hosted guest sessions; dashboard `/api/settings/browser` for runtime limits (idle_timeout_secs, max_concurrent) |

## What it does

Long-lived headless-Chromium tabs the model drives across multiple tool
calls. The same shared Chromium process powers
[`render_page`](html.md), the Google search engine, and the
StackOverflow scraper — opening a session is cheap because the browser
is already running.

There are three distinct concepts in play. Mixing them up is the most
common confusion point with this family:

1. **Session.** One Chromium tab. Identified by an opaque
   `session_id`. The model opens, drives, and closes sessions via the
   first ten tools listed above (`browser_open` … `browser_close`).
   Sessions are ephemeral by default — call `browser_close` when
   you're done so the concurrent cap doesn't fill up.

2. **Persona.** One *named* long-lived tab the model owns —
   `browser_persona_get { name: "google" }` returns a session that
   *reuses* the same tab + cookies on every subsequent call with the
   same name. Cookies, localStorage, solved-CAPTCHA tokens accumulate
   under the name, so the target site sees one persistent user
   instead of N anonymous strangers. This is the rate-limit-relief
   mechanism. Personas are never auto-reaped — the operator owns the
   lifecycle via `browser_persona_reset` or the dashboard.

3. **Guest session.** A tab we host on behalf of a constellation
   peer. Created exclusively by the inbound
   `/constellation/browser_persona` endpoint, never visible to local
   MCP tools, namespaced per peer so peer A's `"google"` and peer B's
   `"google"` are isolated browser contexts. SSRF-restricted (see
   [security.md](../security.md#browser-sandbox)). Reaped when the
   peer leaves the constellation or after `idle_timeout_secs * 2` of
   silence.

Personas vs guest sessions are surfaced as two separate tables on the
dashboard's `/browser` page so the distinction is visible.

## Tools

### Session lifecycle

#### `browser_open`

`{ }` → `{ session_id, url, tip }`

Opens a new isolated Chromium tab and registers it under a fresh
`session_id`. The tab starts at `about:blank`; use `browser_navigate`
to go somewhere. Subject to the `max_concurrent` cap (default 8) —
past that, the call returns an error and an existing session must be
closed first.

#### `browser_navigate`

`{ session_id, url, observe? }` → `{ url, title, observation }`

Navigates the session to `url`. Waits up to 15 s for the navigation
to settle before returning. The final URL after redirects + page
title come back. `observe` is one of `"none"` (default), `"tree"`
(compact accessibility-style tree of interactive elements with stable
selectors), `"screenshot"` (viewport PNG, base64), or `"both"`.

#### `browser_close`

`{ session_id }` → `"closed session <id>"`

Disposes the tab and its isolated `BrowserContext`. Errors on an
unknown id.

#### `browser_list`

`{ }` → `{ sessions: [...] }`

Every open session with `session_id`, `created_secs_ago`,
`idle_secs`, and live `url` + `title`. Useful for finding a lingering
session before opening a new one.

### Interaction

#### `browser_click`

`{ session_id, selector, observe? }` → `{ url, observation }`

Clicks the first element matching the CSS `selector`. Waits 5 s for
any post-click navigation. Returns the resulting URL so a click-that-
navigates and a click-that-stays-put are easy to distinguish.

#### `browser_type`

`{ session_id, selector, text, submit?, observe? }` → `{ url, observation }`

Focuses the selector and types `text`. With `submit: true`, calls
`form.requestSubmit()` and waits 15 s for navigation — a search-box
round-trip in one call.

#### `browser_wait`

`{ session_id, selector, timeout_ms? }` → `{ matched }`

Polls until the selector exists in the DOM or `timeout_ms` elapses
(default 5000, max 60000). Returns `{matched: bool}` so the caller
can branch instead of treating timeout as an error.

#### `browser_extract`

`{ session_id, selector, attr?, limit? }` → `{ values }`

Returns `innerText` (or an attribute when `attr` is set) for every
match. Capped at `limit` (default 50, max 500) so a giant list
doesn't blow the response.

### Observation

#### `browser_eval`

`{ session_id, script }` → `{ result }`

Arbitrary JS expression evaluated in the page; result is JSON.
Promises are awaited. Wrap multi-statement scripts in an IIFE.
Refused on guest sessions (raw `fetch()` would bypass URL guards).

#### `browser_screenshot`

`{ session_id, full_page? }` → `{ png_b64 }`

PNG of the viewport, or full scroll height with `full_page: true`.

### Personas

#### `browser_persona_get`

`{ name }` → `{ session_id, state }`

Returns the session id of the named local persona, creating it on
first call. Subsequent calls with the same name return the same id
(cookies persist). `state` is `"healthy"`, `"suspect"`, or
`"blocked"`.

#### `browser_persona_list`

`{ }` → `{ personas: [...] }`

Lists local personas only. Guest sessions are dashboard-only.

#### `browser_persona_reset`

`{ name }` → `{ session_id, state: "healthy" }`

Disposes the persona's current session + context and creates a fresh
one. State returns to `healthy`.

#### `browser_persona_delegate`

`{ persona_name, url }` → `{ url, title, tree }`

Asks a constellation peer (one with `[network.capabilities].browser
= true`) to run a navigate on ITS named persona and return a compact
observation. Sessions don't transport — the peer uses its own warm
state on its own IP. Use this when the local persona is `blocked`
(CAPTCHA stuck / rate-limit) and you want to try the same query
from a different network.

## Observation tree

The `observe: "tree"` parameter on every page-driving tool returns a
compact list of interactive DOM elements:

```json
{
  "tree": [
    { "role": "textbox", "name": "Search", "selector": "[name='q']", "value": "" },
    { "role": "button", "name": "I'm Feeling Lucky", "selector": "[name='btnI']" },
    { "role": "link", "name": "About", "selector": "a:nth-of-type(1)" }
  ]
}
```

Selector priority: `#id` > `[data-testid=...]` > a nth-of-type CSS
path back to the nearest stable ancestor. Capped at 150 nodes so a
pathological page can't blow the response.

## State machine for personas

| State | Trigger | Effect |
| --- | --- | --- |
| `healthy` | initial; or after `browser_persona_reset` | Use freely. |
| `suspect` | first match of CAPTCHA / 429 / 403 / "just a moment" / "access denied" in URL or title | Use still works; the model should consider backing off or routing to a different provider. |
| `blocked` | second match | `browser_persona_get` returns an error. Operator must reset from the dashboard. |

The detector runs after every `browser_navigate`; warnings are
advisory only.

## Guest sessions (peer-hosted)

Created exclusively by the inbound `/constellation/browser_persona`
endpoint when a peer asks us to drive their named persona. Stored in
a separate registry keyed by `(peer_id, persona_name)` so each peer's
state is isolated. Always SSRF-restricted (see
[security.md](../security.md#browser-sandbox)):

- `browser_navigate` rejects URLs whose host is RFC1918 / loopback /
  link-local / CGNAT / IPv6 ULA, plus `.local` / `.lan` / `.internal`
  / `.home.arpa` / `.test` TLDs.
- `browser_eval` is rejected outright (raw `fetch()` would bypass the
  URL guard).
- Click / type / submit that lands on a private host rolls the page
  back to `about:blank` and returns an error.

Reaped when the peer drops out of the peer table
(`evict_guest_sessions_for_peer`) or after `idle_timeout_secs * 2` of
silence with no live underlying session.

Guest sessions never appear in `browser_persona_list` and aren't
addressable by any MCP tool. The operator sees them on the dashboard
under "Hosted for peers".

## Settings (runtime-tunable from the dashboard)

`POST /api/settings/browser` (Bearer auth against `[network].token`):

| Field | Range | Default | Meaning |
| --- | --- | --- | --- |
| `idle_timeout_secs` | 30 – 86 400 | 1800 (30 min) | Session reaped after this long without a tool call. Guest sessions use `* 2`. |
| `max_concurrent` | 1 – 64 | 8 | Cap on simultaneously open sessions across the whole server (local + guest combined). |

The dashboard's `/browser` page surfaces both.

## Master switch for hosting guest sessions

`[network.capabilities].browser = false` (the default) refuses every
inbound `/constellation/browser_persona` request with `403 disabled`.
With the cap off, the "Hosted for peers" table stays empty by
construction. The operator can flip the cap at runtime from the
constellation settings drawer — no restart needed.

## See also

- [`docs/security.md`](../security.md) — full trust-boundary +
  control-surface reference, including the SSRF policy details.
- [`docs/constellation.md`](../constellation.md) — capabilities,
  delegation, peer-departure cleanup.
- [`docs/skills/html.md`](html.md) — the one-shot `html_render` tool
  for rendering HTML or a URL with diagnostics (console, exceptions,
  network failures). Use this when you don't need a long-lived tab.
