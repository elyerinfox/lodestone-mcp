//! Token-counting skills (local compute): count tokens against OpenAI BPE
//! tokenizer families and compare across models. LLMs cannot count their own
//! tokens accurately; this gives the model a deterministic answer for
//! cost / context-window planning.
//!
//! Supported tokenizers (via `tiktoken-rs`):
//! - `cl100k_base` (GPT-3.5-turbo, GPT-4, text-embedding-3-small/large)
//! - `o200k_base` (GPT-4o, GPT-4o-mini, o1, o3)
//! - `p50k_base` (text-davinci-002/003, code-davinci-002)
//! - `r50k_base` (legacy GPT-3 davinci/curie/babbage/ada)
//!
//! Anthropic / Llama / Mistral families do NOT publish their tokenizers in
//! a vendoring-friendly form; for those models the count is approximated
//! via cl100k_base with a clear caveat in the response.
//!
//! ## Sources
//!
//! - `tiktoken-rs` crate (port of OpenAI's `tiktoken`).
//! - OpenAI model → tokenizer mapping per the
//!   [tiktoken model.py](https://github.com/openai/tiktoken/blob/main/tiktoken/model.py).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;
use tiktoken_rs::{cl100k_base, o200k_base, p50k_base, r50k_base, CoreBPE};

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// token_count
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CountArgs {
    /// Text to tokenize.
    text: String,
    /// Model alias OR tokenizer name. Aliases: `gpt-4o`, `gpt-4o-mini`,
    /// `gpt-4`, `gpt-4-turbo`, `gpt-3.5-turbo`, `o1`, `o3`, `claude`
    /// (approximated), `llama` (approximated). Direct tokenizers:
    /// `cl100k_base`, `o200k_base`, `p50k_base`, `r50k_base`. Defaults to
    /// `gpt-4o` (i.e. `o200k_base`).
    #[serde(default)]
    model: Option<String>,
}

pub struct TokenCount;
impl Skill for TokenCount {
    fn name(&self) -> &'static str {
        "token_count"
    }
    fn description(&self) -> &'static str {
        "Count tokens for `text` against the chosen tokenizer family. Defaults to `gpt-4o` \
         (o200k_base). Returns the token count, character count, and a tokens-per-char ratio \
         useful for budgeting. Anthropic / Llama / Mistral aren't supported natively — the call \
         falls back to cl100k_base with a `caveat` field explaining the approximation."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CountArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<CountArgs>()?;
            let model = args.model.as_deref().unwrap_or("gpt-4o").trim();
            let (bpe, tokenizer_name, caveat) = resolve_tokenizer(model)?;
            let tokens = bpe.encode_with_special_tokens(&args.text);
            let count = tokens.len();
            let mut obj = json!({
                "model": model,
                "tokenizer": tokenizer_name,
                "token_count": count,
                "char_count": args.text.chars().count(),
                "tokens_per_char": if args.text.is_empty() {
                    0.0
                } else {
                    (count as f64 / args.text.chars().count() as f64 * 1000.0).round() / 1000.0
                },
            });
            if let Some(c) = caveat {
                obj["caveat"] = json!(c);
            }
            Ok(text_result(obj.to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "GPT-4o default",
                args: r#"{"text": "Hello, world!"}"#,
                note: Some("Uses o200k_base — the GPT-4o / o1 / o3 tokenizer."),
            },
            SkillExample {
                title: "GPT-4 / GPT-3.5",
                args: r#"{"text": "Hello, world!", "model": "gpt-4"}"#,
                note: Some("Uses cl100k_base."),
            },
            SkillExample {
                title: "Direct tokenizer name",
                args: r#"{"text": "Hello, world!", "model": "cl100k_base"}"#,
                note: None,
            },
            SkillExample {
                title: "Claude approximation (with caveat)",
                args: r#"{"text": "Hello, world!", "model": "claude"}"#,
                note: Some("Anthropic doesn't publish a usable tokenizer; falls back to cl100k_base with a caveat."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate API cost before sending a prompt to GPT-4o / GPT-4 / o1.",
            "Decide how to chunk a long document to fit in a context window.",
            "Compare two prompt variants for token efficiency.",
        ]
    }
}

// ---------------------------------------------------------------------------
// token_compare
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CompareArgs {
    /// Text to tokenize against every supported tokenizer family.
    text: String,
}

pub struct TokenCompare;
impl Skill for TokenCompare {
    fn name(&self) -> &'static str {
        "token_compare"
    }
    fn description(&self) -> &'static str {
        "Tokenize `text` against every supported tokenizer family (o200k_base, cl100k_base, \
         p50k_base, r50k_base) and report the token counts side-by-side. Useful when picking \
         between GPT-4o (o200k) and GPT-4 (cl100k) for cost-per-1k-token planning."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CompareArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<CompareArgs>()?;
            let entries: Vec<(&str, &str, CoreBPE)> = vec![
                (
                    "o200k_base",
                    "gpt-4o / o1 / o3",
                    o200k_base().map_err(internal_err)?,
                ),
                (
                    "cl100k_base",
                    "gpt-4 / gpt-3.5-turbo / text-embedding-3-*",
                    cl100k_base().map_err(internal_err)?,
                ),
                (
                    "p50k_base",
                    "text-davinci-002/003 / code-davinci-002",
                    p50k_base().map_err(internal_err)?,
                ),
                (
                    "r50k_base",
                    "legacy gpt-3 (davinci/curie/babbage/ada)",
                    r50k_base().map_err(internal_err)?,
                ),
            ];
            let rows: Vec<serde_json::Value> = entries
                .iter()
                .map(|(name, models, bpe)| {
                    let count = bpe.encode_with_special_tokens(&args.text).len();
                    json!({
                        "tokenizer": name,
                        "models": models,
                        "token_count": count,
                    })
                })
                .collect();
            Ok(text_result(
                json!({
                    "char_count": args.text.chars().count(),
                    "comparison": rows,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Side-by-side comparison",
            args: r#"{"text": "The quick brown fox jumps over the lazy dog."}"#,
            note: Some("Returns one row per tokenizer with the count + which models use it."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pick the most token-efficient model for a given prompt.",
            "See how a prompt's token count differs across model generations.",
        ]
    }
}

fn internal_err(e: anyhow::Error) -> McpError {
    invalid(format!("tiktoken: {e}"))
}

fn resolve_tokenizer(
    model: &str,
) -> Result<(CoreBPE, &'static str, Option<&'static str>), McpError> {
    let lower = model.to_ascii_lowercase();
    // Direct tokenizer names.
    match lower.as_str() {
        "cl100k_base" => return Ok((cl100k_base().map_err(internal_err)?, "cl100k_base", None)),
        "o200k_base" => return Ok((o200k_base().map_err(internal_err)?, "o200k_base", None)),
        "p50k_base" => return Ok((p50k_base().map_err(internal_err)?, "p50k_base", None)),
        "r50k_base" => return Ok((r50k_base().map_err(internal_err)?, "r50k_base", None)),
        _ => {}
    }
    // Model alias → tokenizer family.
    // Mapping mirrors tiktoken's model.py.
    let bpe_name = if lower.starts_with("gpt-4o")
        || lower.starts_with("o1")
        || lower.starts_with("o3")
    {
        ("o200k_base", None)
    } else if lower.starts_with("gpt-4")
        || lower.starts_with("gpt-3.5")
        || lower.starts_with("text-embedding-3")
        || lower.starts_with("text-embedding-ada-002")
    {
        ("cl100k_base", None)
    } else if lower.starts_with("text-davinci-002")
        || lower.starts_with("text-davinci-003")
        || lower.starts_with("code-davinci")
    {
        ("p50k_base", None)
    } else if lower.starts_with("davinci")
        || lower.starts_with("curie")
        || lower.starts_with("babbage")
        || lower.starts_with("ada")
    {
        ("r50k_base", None)
    } else if lower.starts_with("claude") || lower.starts_with("anthropic") {
        (
            "cl100k_base",
            Some("Anthropic does not publish a usable tokenizer; this count is an approximation via cl100k_base. Actual Claude tokenization differs."),
        )
    } else if lower.starts_with("llama")
        || lower.starts_with("mistral")
        || lower.starts_with("mixtral")
        || lower.starts_with("phi")
        || lower.starts_with("gemma")
    {
        (
            "cl100k_base",
            Some("This model uses its own SentencePiece tokenizer not bundled here; this count is approximated via cl100k_base."),
        )
    } else {
        return Err(invalid(format!(
            "unknown model `{model}`. Try gpt-4o, gpt-4, gpt-3.5-turbo, o1, o3, claude, llama, or a direct tokenizer name (cl100k_base / o200k_base / p50k_base / r50k_base)."
        )));
    };
    let bpe = match bpe_name.0 {
        "o200k_base" => o200k_base().map_err(internal_err)?,
        "cl100k_base" => cl100k_base().map_err(internal_err)?,
        "p50k_base" => p50k_base().map_err(internal_err)?,
        "r50k_base" => r50k_base().map_err(internal_err)?,
        _ => unreachable!(),
    };
    Ok((bpe, bpe_name.0, bpe_name.1))
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "token"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Token counting against OpenAI tokenizer families (cl100k_base, o200k_base, p50k_base, \
         r50k_base) via tiktoken-rs. Pure local compute. Anthropic / Llama / Mistral get an \
         approximate count with a clear caveat — they don't publish vendoring-friendly tokenizers."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `token_count { text: \"<prompt>\", model: \"gpt-4o\" }` — how many tokens against o200k_base?\n\
             2. `token_compare { text: \"<prompt>\" }` — see counts side-by-side across every tokenizer to pick the most efficient.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(TokenCount), Box::new(TokenCompare)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cl100k_loads_and_counts() {
        let bpe = cl100k_base().unwrap();
        assert!(!bpe.encode_with_special_tokens("Hello, world!").is_empty());
    }

    #[test]
    fn resolve_gpt_4o_alias() {
        let (_, name, _) = resolve_tokenizer("gpt-4o").unwrap();
        assert_eq!(name, "o200k_base");
    }

    #[test]
    fn resolve_gpt_4_alias() {
        let (_, name, _) = resolve_tokenizer("gpt-4").unwrap();
        assert_eq!(name, "cl100k_base");
    }

    #[test]
    fn resolve_claude_has_caveat() {
        let (_, name, caveat) = resolve_tokenizer("claude-3-opus").unwrap();
        assert_eq!(name, "cl100k_base");
        assert!(caveat.is_some());
    }

    #[test]
    fn unknown_model_errors() {
        assert!(resolve_tokenizer("not-a-model-x").is_err());
    }
}
