# Structured data — `json_query` / `json_format` / `yaml_to_json` / `json_to_yaml`

|  |  |
| --- | --- |
| **Module** | [`src/skills/data.rs`](../../src/skills/data.rs) |
| **Tools** | `json_query`, `json_format`, `yaml_to_json`, `json_to_yaml` |
| **Network** | local-only |
| **Default** | on |
| **Config** | none |

## What it does
Parses, validates, searches, and serializes JSON and YAML — entirely local, no network.
`json_query` validates JSON and optionally extracts a value by JSON Pointer;
`json_format` pretty-prints or minifies; `yaml_to_json` / `json_to_yaml` convert between
the two formats. Every tool validates its input and reports parse errors.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `json_query` | `json`, `pointer?` | Parse/validate JSON; extract a value by RFC-6901 JSON Pointer (e.g. `/items/0/name`), or, without a pointer, return the whole document pretty-printed. |
| `json_format` | `json`, `minify?` | Reformat JSON: pretty-print (default) or minify (`minify=true`). |
| `yaml_to_json` | `data` | Convert a YAML document to pretty-printed JSON. |
| `json_to_yaml` | `data` | Convert a JSON document to YAML. |

## Configuration & gating
No configuration. Each tool is independently gateable via `[tools]`.

## Example uses
- **Pull one field** — `json_query` with `pointer=/items/0/name` out of an API response.
- **Validate + tidy** — `json_format` to confirm a payload parses and pretty-print it.
- **Bridge formats** — `yaml_to_json` a Kubernetes manifest, then `json_query` into it.

## See also
[tools.md](../tools.md)
