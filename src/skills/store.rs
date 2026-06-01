//! File-store and cache-management skills.
//!
//! The `store_*` tools manage the on-disk [`crate::store::FileStore`] (gated by
//! `[store]`, off by default): fetch a URL's bytes into it, read/list/purge entries.
//! `cache_status` reports the in-memory caches and the store, and is always
//! available (read-only).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::{human_size, truncate_chars};
use crate::{internal, invalid, text_result};

/// Store tools gated by `[store].enabled`. (`cache_status` is intentionally not
/// listed — it stays available to report whatever caches exist.)

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FetchArgs {
    /// URL to download and cache in the file store.
    url: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct KeyArgs {
    /// The entry key (the URL it was fetched/stored under).
    key: String,
    /// Max characters of text to return. Omit for the server default.
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PurgeArgs {
    /// A specific entry key to remove. Omit to purge the entire store.
    #[serde(default)]
    key: Option<String>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for `store_purge` for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

fn store_of(server: &crate::Lodestone) -> Result<&Arc<crate::store::FileStore>, McpError> {
    server.store.as_ref().ok_or_else(|| {
        invalid("the file store is disabled — enable it with [store].enabled = true")
    })
}

pub struct StoreFetch;
impl Skill for StoreFetch {
    fn name(&self) -> &'static str {
        "store_fetch"
    }
    fn description(&self) -> &'static str {
        "Cache a URL's bytes in the on-disk file store, returning the local path and size. Dodges \
        the source when possible: a local copy → a constellation peer that has it → finally the source. \
        Useful for rate-limited downloads (e.g. arXiv/IETF PDFs) shared across a constellation. Use \
        store_get to read it back."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FetchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<FetchArgs>()?;
            let store = store_of(server)?;
            let url = args.url.trim();
            // Shared fetch: local store → constellation peer → source (caches the result).
            let bytes = server.fetch_bytes_shared(url).await.map_err(internal)?;
            let path = store.path_for(url);
            Ok(text_result(format!(
                "Stored {} ({}) at {}",
                url,
                human_size(bytes.len() as u64),
                path.display()
            )))
        })
    }
}

pub struct StoreGet;
impl Skill for StoreGet {
    fn name(&self) -> &'static str {
        "store_get"
    }
    fn description(&self) -> &'static str {
        "Read a stored entry's content as text (UTF-8 lossy, truncated), by its key (the URL it was \
        stored under). For binary entries the text may be garbled — prefer this for pages/JSON/text."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<KeyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<KeyArgs>()?;
            let store = store_of(server)?;
            match store.get(args.key.trim()).await {
                Some(bytes) => {
                    let max = server.clamp_chars(args.max_chars);
                    let text = String::from_utf8_lossy(&bytes);
                    Ok(text_result(truncate_chars(&text, max)))
                }
                None => Ok(text_result(format!(
                    "No stored entry for '{}' (missing or expired).",
                    args.key.trim()
                ))),
            }
        })
    }
}

pub struct StoreList;
impl Skill for StoreList {
    fn name(&self) -> &'static str {
        "store_list"
    }
    fn description(&self) -> &'static str {
        "List entries in the on-disk file store (key/URL, size, age), newest first."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            let store = store_of(server)?;
            let entries = store.list().await;
            if entries.is_empty() {
                return Ok(text_result("File store is empty."));
            }
            let total: u64 = entries.iter().map(|e| e.size).sum();
            let mut out = format!(
                "File store: {} entr{} ({})\n",
                entries.len(),
                if entries.len() == 1 { "y" } else { "ies" },
                human_size(total)
            );
            for e in &entries {
                let age = e
                    .modified
                    .elapsed()
                    .map(|d| format_age(d.as_secs()))
                    .unwrap_or_else(|_| "?".into());
                // Seed ratio (served vs. fetched over the constellation), if tracked.
                let seed = server
                    .registry
                    .blob_seed_ratio(&crate::constellation::hash_key(&e.key))
                    .map(|(served, fetched, ratio)| {
                        let r = ratio
                            .map(|r| format!("{r:.2}"))
                            .unwrap_or_else(|| "∞".to_string());
                        format!(", seed ↑{served}/↓{fetched} ratio {r}")
                    })
                    .unwrap_or_default();
                out.push_str(&format!(
                    "\n  {} ({}, {age} ago{seed})",
                    e.key,
                    human_size(e.size)
                ));
            }
            Ok(text_result(out))
        })
    }
}

pub struct StorePurge;
impl Skill for StorePurge {
    fn name(&self) -> &'static str {
        "store_purge"
    }
    fn description(&self) -> &'static str {
        "Remove a file-store entry by key, or purge the whole store when no key is given. \
        **Destructive** — deletes cached bytes on disk. First call returns a confirmation token \
        and does nothing; call again with `confirm=<token>` to delete (or `confirm + trust=true`). \
        `[store].allow_destructive=true` pre-authorizes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PurgeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use crate::skills::guard::Decision;
            let (server, args) = ctx.parse::<PurgeArgs>()?;
            let store = store_of(server)?;
            let key_clean = args.key.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let summary = match key_clean {
                Some(key) => format!("remove '{key}' from the file store"),
                None => "purge the entire file store".to_string(),
            };
            if let Decision::Challenge(msg) = server.guard.check(
                "store_purge",
                "store_purge",
                server.cfg.store.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            match key_clean {
                Some(key) => {
                    let existed = store.remove(key).await;
                    Ok(text_result(if existed {
                        format!("Removed '{key}' from the store.")
                    } else {
                        format!("No stored entry for '{key}'.")
                    }))
                }
                None => {
                    let n = store.purge().await;
                    Ok(text_result(format!(
                        "Purged the store ({n} entr{} removed).",
                        if n == 1 { "y" } else { "ies" }
                    )))
                }
            }
        })
    }
}

pub struct CacheStatus;
impl Skill for CacheStatus {
    fn name(&self) -> &'static str {
        "cache_status"
    }
    fn description(&self) -> &'static str {
        "Report the state of the caches: the in-memory search-result cache, the retrieval-output \
        cache, and the on-disk file store (entry counts and total size). Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let server = ctx.server;
            let mut out = String::from("Caches:\n");
            match server.registry.cache_len() {
                Some(n) => out.push_str(&format!("  search cache: {n} live entr{}\n", plural(n))),
                None => out.push_str("  search cache: disabled\n"),
            }
            match server.retrieval_cache.as_ref().map(|c| c.keys().len()) {
                Some(n) => {
                    out.push_str(&format!("  retrieval cache: {n} live entr{}\n", plural(n)))
                }
                None => out.push_str("  retrieval cache: disabled\n"),
            }
            match &server.store {
                Some(s) => {
                    let entries = s.list().await;
                    let total: u64 = entries.iter().map(|e| e.size).sum();
                    out.push_str(&format!(
                        "  file store: {} entr{} ({}) at {}\n",
                        entries.len(),
                        plural(entries.len()),
                        human_size(total),
                        s.dir().display()
                    ));
                }
                None => out.push_str("  file store: disabled\n"),
            }
            Ok(text_result(out))
        })
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        "y"
    } else {
        "ies"
    }
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// The skills this module contributes. `cache_status` is always present; the
/// `store_*` tools are gated by `[store]` in `disabled_by_config`.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(StoreFetch),
        Box::new(StoreGet),
        Box::new(StoreList),
        Box::new(StorePurge),
        Box::new(CacheStatus),
    ]
}

#[cfg(test)]
mod tests {
    use super::format_age;

    #[test]
    fn ages_format() {
        assert_eq!(format_age(5), "5s");
        assert_eq!(format_age(120), "2m");
        assert_eq!(format_age(7_200), "2h");
        assert_eq!(format_age(172_800), "2d");
    }
}
