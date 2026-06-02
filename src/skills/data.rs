//! Structured-data skills (local, no network): parse / search / serialize JSON
//! and YAML. `json_query` validates JSON and optionally extracts a value by JSON
//! Pointer; `json_format` pretty-prints or minifies; `yaml_to_json` /
//! `json_to_yaml` convert between the two.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct JsonQueryArgs {
    /// The JSON document (as text).
    json: String,
    /// Optional RFC-6901 JSON Pointer to extract, e.g. `/items/0/name`. Omit to
    /// validate + return the whole document pretty-printed.
    #[serde(default)]
    pointer: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct JsonFormatArgs {
    /// The JSON document (as text).
    json: String,
    /// Minify (compact) instead of pretty-printing (default false = pretty).
    #[serde(default)]
    minify: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConvertArgs {
    /// The document to convert (as text).
    data: String,
}

pub struct JsonQuery;
impl Skill for JsonQuery {
    fn name(&self) -> &'static str {
        "json_query"
    }
    fn description(&self) -> &'static str {
        "Parse/validate JSON and optionally extract a value by RFC-6901 JSON Pointer (e.g. \
        `/items/0/name`). Without a pointer, returns the whole document pretty-printed. Local, no \
        network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<JsonQueryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<JsonQueryArgs>()?;
            let v: Value = serde_json::from_str(&args.json)
                .map_err(|e| invalid(format!("invalid JSON: {e}")))?;
            let selected = match args
                .pointer
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(ptr) => v
                    .pointer(ptr)
                    .ok_or_else(|| invalid(format!("no value at JSON Pointer '{ptr}'")))?,
                None => &v,
            };
            Ok(text_result(
                serde_json::to_string_pretty(selected).unwrap_or_default(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Validate JSON without selection",
                args: r#"{"json": "{\"name\":\"alice\",\"age\":30}"}"#,
                note: Some("Returns the whole document pretty-printed."),
            },
            SkillExample {
                title: "Extract by JSON Pointer",
                args: r#"{"json": "{\"items\":[{\"name\":\"a\"},{\"name\":\"b\"}]}", "pointer": "/items/0/name"}"#,
                note: Some("Returns `\"a\"`. Errors if the pointer doesn't resolve."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pull a single nested field out of a large JSON blob.",
            "Parse-check JSON returned from another tool before passing it on.",
            "Pretty-print a minified JSON string for human inspection.",
        ]
    }
}

pub struct JsonFormat;
impl Skill for JsonFormat {
    fn name(&self) -> &'static str {
        "json_format"
    }
    fn description(&self) -> &'static str {
        "Reformat a JSON document: pretty-print (default) or minify (`minify=true`). Validates the \
        input. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<JsonFormatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<JsonFormatArgs>()?;
            let v: Value = serde_json::from_str(&args.json)
                .map_err(|e| invalid(format!("invalid JSON: {e}")))?;
            let out = if args.minify.unwrap_or(false) {
                serde_json::to_string(&v)
            } else {
                serde_json::to_string_pretty(&v)
            }
            .unwrap_or_default();
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Pretty-print",
                args: r#"{"json": "{\"a\":1,\"b\":[2,3]}"}"#,
                note: Some("Default behavior; minify=false."),
            },
            SkillExample {
                title: "Minify",
                args: r#"{"json": "{\n  \"a\": 1\n}", "minify": true}"#,
                note: Some("Returns the compact form."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Reflow a pasted JSON blob into a canonical pretty layout.",
            "Compact JSON for embedding or transmission.",
            "Validate JSON syntax without extracting any specific field.",
        ]
    }
}

pub struct YamlToJson;
impl Skill for YamlToJson {
    fn name(&self) -> &'static str {
        "yaml_to_json"
    }
    fn description(&self) -> &'static str {
        "Convert a YAML document to pretty-printed JSON. Validates the YAML. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConvertArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ConvertArgs>()?;
            let v: Value = serde_yaml::from_str(&args.data)
                .map_err(|e| invalid(format!("invalid YAML: {e}")))?;
            Ok(text_result(
                serde_json::to_string_pretty(&v).unwrap_or_default(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Convert a small YAML",
                args: r#"{"data": "name: alice\nage: 30\n"}"#,
                note: Some("Returns pretty-printed JSON."),
            },
            SkillExample {
                title: "Kubernetes-style manifest",
                args: r#"{"data": "apiVersion: v1\nkind: Pod\nmetadata:\n  name: web\n"}"#,
                note: Some("Preserves nested mapping structure."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Feed YAML config into a tool that only accepts JSON.",
            "Validate YAML by round-tripping through serde_json.",
            "Normalize a YAML doc into JSON for diffing.",
        ]
    }
}

pub struct JsonToYaml;
impl Skill for JsonToYaml {
    fn name(&self) -> &'static str {
        "json_to_yaml"
    }
    fn description(&self) -> &'static str {
        "Convert a JSON document to YAML. Validates the JSON. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConvertArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ConvertArgs>()?;
            let v: Value = serde_json::from_str(&args.data)
                .map_err(|e| invalid(format!("invalid JSON: {e}")))?;
            let yaml = serde_yaml::to_string(&v)
                .map_err(|e| invalid(format!("could not serialize to YAML: {e}")))?;
            Ok(text_result(yaml))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Object to YAML",
                args: r#"{"data": "{\"name\":\"alice\",\"age\":30}"}"#,
                note: Some("Returns YAML; nested mappings and lists preserved."),
            },
            SkillExample {
                title: "Array roundtrip",
                args: r#"{"data": "[1, 2, 3]"}"#,
                note: Some("Top-level arrays become a YAML sequence."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert API responses into YAML for human readability.",
            "Generate sample config files from a JSON template.",
            "Diff config in YAML form when the source was JSON.",
        ]
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(JsonQuery),
        Box::new(JsonFormat),
        Box::new(YamlToJson),
        Box::new(JsonToYaml),
    ]
}
