# Language — `translate` / `detect_language`

|  |  |
| --- | --- |
| **Module** | [`src/skills/translate.rs`](../../src/skills/translate.rs) |
| **Tools** | `translate`, `detect_language` |
| **Network** | keyless API (Google Translate) |
| **Default** | on |
| **Config** | none — hits a keyless Google Translate endpoint; results are cached |

## What it does
Translates text and detects languages via the public Google Translate
`translate_a/single` endpoint that backs the Translate web widget — a plain keyless
GET, no API key or account. It transforms text rather than returning a ranked list,
so it's a pair of standalone skills, not a search provider.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `translate` | `text`, `to`, `from?` | Translate `text` into the `to` ISO-639 code (e.g. `es`, `fr`, `de`, `ja`, `zh-CN`); `from` defaults to auto-detect. Returns the translation and the detected source language. |
| `detect_language` | `text` | Detect the text's language; returns the detected ISO-639 code. |

## Configuration & gating
No configuration. Both tools call the keyless Google Translate endpoint and cache the
result keyed on the input (so repeated requests don't re-hit the network). Each tool is
independently gateable via `[tools]`.

## Example uses
- **Translate a snippet** — `translate` a log line or doc paragraph into English.
- **Localize output** — `translate` a reply into the user's language (`to=ja`).
- **Route by language** — `detect_language` first, then branch on the returned code.

## See also
[tools.md](../tools.md)
