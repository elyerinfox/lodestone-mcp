//! Skills (tools) — the uniform capability layer.
//!
//! Every tool the server exposes is a **skill**: a self-contained module here that
//! implements the [`Skill`] contract (`name` / `description` / `schema` / `call`).
//! `main.rs` holds no tool logic — it builds shared state ([`crate::Lodestone`])
//! and assembles the router from [`all_routes`]. A skill's own domain logic (API/
//! socket clients, parsers, formatters) lives *in its module*, not at the crate
//! root. Data sources implement [`crate::provider::SearchProvider`] under
//! `src/providers/`; skills may build on them.
//!
//! See [golden rule 7](../../docs/golden-rules.md) and the terminology note in
//! [CONTRIBUTING.md](../../CONTRIBUTING.md).

pub mod algebra;
pub mod archive;
pub mod arithmetic;
pub mod artifacthub;
pub mod arxiv;
pub mod astro;
pub mod binary;
pub mod browser_session;
pub mod chart;
pub mod data;
pub mod databases;
pub mod datetime;
pub mod disasm;
pub mod docker;
pub mod eia;
pub mod fcc;
pub mod ffmpeg;
pub mod filesystem;
pub mod finance;
pub mod forecast;
pub mod formula;
pub mod geometry;
pub mod git;
pub mod github;
pub mod grid;
pub mod guard;
pub mod html;
pub mod huggingface;
pub mod image;
pub mod kernel;
pub mod kubernetes;
pub mod memory;
pub mod meta;
pub mod nasa;
pub mod news;
pub mod noaa;
pub mod notebook;
pub mod oci;
pub mod openaccess;
pub mod osm;
pub mod pcap;
pub mod peeringdb;
pub mod physics;
pub mod printer;
pub mod pubmed;
pub mod python;
pub mod radio;
pub mod regex;
pub mod retrieve;
pub mod rfc;
pub mod satellite;
pub mod sdr;
pub mod search;
pub mod serial;
pub mod shell;
pub mod signal;
pub mod spreadsheet;
pub mod ssrf;
pub mod standards;
pub mod stocks;
pub mod store;
pub mod sysinfo;
pub mod systemd;
pub mod tasks;
pub mod translate;
pub mod trigonometry;
pub mod units;
pub mod wave;
pub mod weather;
pub mod wikipedia;
pub mod yahoo;

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::handler::server::router::tool::ToolRoute;
use rmcp::handler::server::tool::{parse_json_object, schema_for_type, ToolCallContext};
use rmcp::model::{CallToolResult, JsonObject, Tool};
use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

use crate::Lodestone;

/// What a [`Skill::call`] receives: the shared server state plus the raw,
/// already-extracted argument object (parse it with [`SkillCtx::parse`]).
pub struct SkillCtx<'a> {
    pub server: &'a Lodestone,
    pub args: JsonObject,
}

impl<'a> SkillCtx<'a> {
    /// Parse the arguments into a typed struct, returning the server handle too.
    pub fn parse<T: DeserializeOwned>(self) -> Result<(&'a Lodestone, T), McpError> {
        let args = parse_json_object::<T>(self.args)?;
        Ok((self.server, args))
    }
}

/// The contract every tool implements. Object-safe, so skills are stored as
/// `Box<dyn Skill>` and assembled uniformly.
pub trait Skill: Send + Sync + 'static {
    /// Tool name (the MCP `name`, e.g. `translate`).
    fn name(&self) -> &'static str;
    /// One-line tool description shown to the model.
    fn description(&self) -> &'static str;
    /// JSON schema of the tool's arguments.
    fn schema(&self) -> Arc<JsonObject>;
    /// Run the tool.
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>>;
}

/// Build a JSON schema for an arguments struct (helper for [`Skill::schema`]).
pub(crate) fn schema_for<T: JsonSchema + 'static>() -> Arc<JsonObject> {
    schema_for_type::<T>()
}

// ---------------------------------------------------------------------------
// Capability framework — per-family runtime probes.
// ---------------------------------------------------------------------------
//
// A skill family's *config* flag (`[<family>].enabled`) says the operator wants
// the tools exposed. A separate *capability* check answers "does this host
// have what the family actually needs to run?" — a Docker daemon socket, a
// reachable kubeconfig, `python3` on `$PATH`, `ffmpeg` on `$PATH`, NVML for
// `system_gpu`, etc. Both gates are independent: a tool only fires when its
// family is enabled in config AND the capability probe returned `Ready`.
//
// The signal flows in three directions:
//   1. The dispatch wrapper turns a missing capability into a clean
//      `invalid_request` error with the reason + a one-line hint. That's what
//      the LLM sees.
//   2. Each missing capability is logged once at startup at `WARN`.
//   3. The WS snapshot carries the per-family status so the dashboard's
//      Tools page can render a badge + collapsed-by-default group with the
//      reason inline.
//
// Probes are stateless: they look at the host (env vars, `$PATH`, file
// existence, OS) — not at the resolved server config. Anything config-driven
// is the operator's choice and stays gated by the family's `enabled` flag.

#[derive(Debug, Clone)]
pub enum SkillCapability {
    /// Probe succeeded — the family's tools can run.
    Ready,
    /// Probe failed; the family's tools are blocked at dispatch and the
    /// dashboard groups them under a collapsed "Unavailable" header. Both
    /// strings are short enough to render inline.
    Unavailable {
        /// One-line description of what's missing, e.g.
        /// `"Docker daemon socket not reachable"`.
        reason: String,
        /// One-line remediation, e.g.
        /// `"mount /var/run/docker.sock or set DOCKER_HOST"`.
        hint: Option<String>,
    },
}

impl SkillCapability {
    pub fn unavailable(reason: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
            hint: Some(hint.into()),
        }
    }
    pub fn unavailable_no_hint(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
            hint: None,
        }
    }
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// The contract every skill **family** implements. Each family-level
/// module under `src/skills/` exports a unit struct (conventionally
/// `pub struct Family;`) that impls this trait, the registry lists it
/// via [`families`], and the dispatch wrapper + dashboard + startup
/// logs all flow off the same source of truth.
///
/// `check_capability` defaults to [`SkillCapability::Ready`] — most
/// families are pure-Rust and have no host requirement. Families that
/// shell out to a binary (`docker`, `git`, `ffmpeg`, …), depend on an
/// OS subsystem (`systemd`, `serial`, `printer`, `sdr`), or need a
/// reachable resource (`kubernetes` kubeconfig, `python` interpreter)
/// override it. The probe runs ONCE at startup; the result is cached.
pub trait FamilyMeta: Send + Sync + 'static {
    /// Stable id used in logs, snapshot fields, dashboard groupings, and
    /// the error messages the LLM sees on a blocked dispatch.
    fn family(&self) -> &'static str;
    /// The tools this family exposes. Lets the registry build the
    /// `tool_name → family` reverse index in one pass.
    fn tools(&self) -> &'static [&'static str];
    /// Probe the host. Default = `Ready` (the common case). Override
    /// when the family needs a binary on `$PATH`, a socket, a
    /// kubeconfig, etc.
    fn check_capability(&self) -> SkillCapability {
        SkillCapability::Ready
    }
}

/// Every family registered with the capability framework. Adding a
/// family means: write its module, register its `Family` here, and the
/// dispatch wrapper + dashboard + startup logs pick it up automatically.
/// Order doesn't matter; the registry is consumed as a set.
///
/// Coverage today: the host-dependent families (those whose tools
/// shell out / touch the OS) are all wired through here. Pure-Rust
/// families (chart, arithmetic, regex, etc.) inherit the default
/// `Ready` and can adopt the trait incrementally — until they do,
/// their tools just behave as before (no gate, no badge change).
pub fn families() -> Vec<Box<dyn FamilyMeta>> {
    vec![
        Box::new(docker::Family),
        Box::new(kubernetes::Family),
        Box::new(python::Family),
        Box::new(systemd::Family),
        Box::new(ffmpeg::Family),
        Box::new(git::Family),
        Box::new(serial::Family),
        Box::new(printer::Family),
        Box::new(sdr::Family),
    ]
}

/// Probe helper used by ~all the families: is `bin` on `$PATH`?
/// Cross-platform — tries `bin` then `bin.exe` / `bin.cmd` on Windows.
pub(crate) fn binary_on_path(bin: &str) -> bool {
    use std::process::Command;
    for candidate in [bin, &format!("{bin}.exe"), &format!("{bin}.cmd")] {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        {
            return true;
        }
    }
    false
}

/// Empty argument set, for skills that take no parameters.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct NoArgs {}

// ---------------------------------------------------------------------------
// Shared building blocks — small helpers many skills used to copy-paste.
// ---------------------------------------------------------------------------

/// Resolve `path` against `[filesystem].roots` and read the file into a byte
/// vector. Returns the canonical resolved `PathBuf` alongside the bytes so
/// callers can render the real path in error messages.
///
/// Centralized so every read-a-file skill (`binary_*`, `image_*`, `disasm_*`,
/// `notebook_*`, `wave_*`, `pcap_*`) uses the same path-resolution policy
/// and the same `read {}: {err}` error formatting.
pub(crate) fn fs_read_bytes(
    server: &crate::Lodestone,
    path: &str,
) -> Result<(std::path::PathBuf, Vec<u8>), McpError> {
    let p = filesystem::resolve(&server.fs, path)?;
    let bytes = std::fs::read(&p)
        .map_err(|e| crate::internal(anyhow::anyhow!("read {}: {e}", p.display())))?;
    Ok((p, bytes))
}

/// Validate that a slice carries at least `min` elements; raise a uniform
/// "needs at least N {what}" `McpError::invalid_params` otherwise. Used by
/// chart / signal / forecast tools that all required this same check.
pub(crate) fn ensure_min_len<T>(items: &[T], min: usize, what: &str) -> Result<(), McpError> {
    if items.len() < min {
        Err(crate::invalid(format!(
            "needs at least {min} {what}, got {}",
            items.len()
        )))
    } else {
        Ok(())
    }
}

/// Build a `reqwest::Client` carrying [`crate::LODESTONE_UA`] for use in
/// `#[ignore]` live integration tests. Replaces ~28 hand-rolled copies of
/// the same `reqwest::Client::builder().user_agent(...).build().unwrap()`
/// pattern across skill modules. Compiled only under `cfg(test)`.
#[cfg(test)]
pub(crate) fn live_http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(crate::LODESTONE_UA)
        .build()
        .unwrap()
}

/// Send a prepared `reqwest::RequestBuilder`, check the HTTP status, and
/// deserialize the body as JSON into `T`. Centralizes the 7-line
/// `.send().await.map_err(internal)?.error_for_status().map_err(internal)?
/// .json().await.map_err(internal)?` ritual that skill modules used to
/// copy at every API call site. Each network / status / parse error is
/// surfaced via [`crate::internal`] with the raw reqwest error message.
///
/// Callers that need special status-code handling (e.g. treating 404 as
/// "not found" rather than an error) should keep their own pipeline —
/// this helper is for the plain "fetch JSON or fail" case. For uniform
/// error prefixing across send/status/decode use [`send_json_ctx`].
#[allow(dead_code)]
pub(crate) async fn send_json<T: serde::de::DeserializeOwned>(
    req: reqwest::RequestBuilder,
) -> Result<T, McpError> {
    let resp = req.send().await.map_err(|e| crate::internal(e.into()))?;
    let resp = resp
        .error_for_status()
        .map_err(|e| crate::internal(e.into()))?;
    resp.json().await.map_err(|e| crate::internal(e.into()))
}

/// Like [`send_json`] but prefixes every error message with `ctx` — useful
/// when callers want a uniform "open-meteo: ...", "nws ...: ..." label
/// across network, status, and decode failures (one `ctx` value, three
/// possible failure sites). Replaces the per-skill `fetch` helpers that
/// duplicated `and_then(error_for_status).map_err(|e| internal(anyhow!("…: {e}")))?`
/// in `noaa::fetch`, `weather::fetch`, and friends.
pub(crate) async fn send_json_ctx<T: serde::de::DeserializeOwned>(
    req: reqwest::RequestBuilder,
    ctx: &str,
) -> Result<T, McpError> {
    let resp = req
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| crate::internal(anyhow::anyhow!("{ctx}: {e}")))?;
    resp.json()
        .await
        .map_err(|e| crate::internal(anyhow::anyhow!("{ctx}: {e}")))
}

/// Extract a "what is the user trying to do" signal from a tool call. Returns
/// `Some(query)` for tools whose arguments naturally carry a free-text question
/// — every search-shaped tool — and `None` for everything else (system
/// operations, math, file paths, …).
///
/// The dispatch wrapper uses this to look up prior recorded solutions
/// intrinsically, so the model gets relevant past work surfaced as preamble
/// without having to call `solution_find` explicitly.
fn intent_trigger(tool_name: &str, args: &JsonObject) -> Option<String> {
    // Skip self-referential / admin tools so we don't recurse or surface
    // recall on a recall.
    if matches!(
        tool_name,
        "solution_find"
            | "solution_record"
            | "solution_show"
            | "solution_list"
            | "solution_update"
            | "solution_forget"
            | "solution_link"
            | "solution_unlink"
            | "solution_graph"
            | "solution_related"
            | "solution_alias_add"
            | "solution_alias_remove"
            | "memory_save"
            | "memory_get"
            | "memory_list"
            | "memory_search"
            | "memory_forget"
            | "synonym_add"
            | "synonym_remove"
            | "synonym_list"
            | "conversation_list"
            | "conversation_show"
            | "conversation_forget"
            | "conversation_prune"
            | "solution_conversations"
            | "remember"
            | "remember_fact"
            | "remember_solution"
            | "recall"
    ) {
        return None;
    }
    // Any tool whose arguments carry a free-text "query" gets recall — this
    // catches the entire search family (web/code/docs/qa/per-provider),
    // wikipedia/arxiv/pubmed/openalex/hf/standards/rfc/news, osm_geocode/
    // osm_overpass, task_run, etc.
    args.get("query")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Render a list of [`memory::RecallHit`] as a compact preamble. When a hit
/// has typed links to other solutions (`supersedes`, `depends-on`,
/// `related-to`, …) those are listed inline so the model can see the local
/// **subgraph** of prior work and decide whether to walk further with
/// `solution_graph` / `solution_related`.
fn recall_preamble(hits: &[memory::RecallHit]) -> String {
    let mut out = format!(
        "💡 {} prior solution{} matching this (advisory — verify before reusing):\n",
        hits.len(),
        if hits.len() == 1 { "" } else { "s" }
    );
    for h in hits {
        let problem: String = h.problem.replace('\n', " ").chars().take(120).collect();
        out.push_str(&format!("  • {} (score {:.1}): {problem}\n", h.id, h.score));
        // If this hit has been superseded, point at the current head loudly
        // *before* the summary — the model should reach for the head, not the
        // obsolete record that happens to match the query.
        if let Some(head) = h.superseded_by_head.as_deref() {
            if head != h.id {
                out.push_str(&format!(
                    "    ⚠ superseded — current head is {head}; prefer it unless you specifically need the older approach\n"
                ));
            }
        }
        if !h.summary.is_empty() {
            let s: String = h.summary.replace('\n', " ").chars().take(160).collect();
            out.push_str(&format!("    summary: {s}\n"));
        }
        // When the dispatch wrapper auto-attached the query as a phrasing,
        // surface that visibly so the model knows the system is *learning*
        // from this interaction — and so a future operator audit of
        // solution_show can trace where each phrasing came from.
        if h.auto_attached_as_phrasing {
            out.push_str(
                "    ✎ noted this phrasing on the solution for next time (auto-aliased)\n",
            );
        }
        if !h.links.is_empty() {
            let mut edges: Vec<String> = h
                .links
                .iter()
                .map(|(kind, to)| format!("─{kind}→ {to}"))
                .collect();
            edges.dedup();
            out.push_str(&format!("    links: {}\n", edges.join("  ")));
            out.push_str(&format!(
                "    ↳ solution_graph id=\"{}\" to walk further, solution_related id=\"{}\" for ranked neighbors\n",
                h.id, h.id
            ));
        } else {
            out.push_str(&format!(
                "    ↳ solution_show id=\"{}\" for full history\n",
                h.id
            ));
        }
    }
    out.push_str("───\n");
    out
}

/// Turn one boxed skill into a dynamic tool route. The wrapper adds two
/// intrinsic behaviors when memory is enabled:
///
/// 1. **Prior-solution recall** — if the tool's arguments carry a query,
///    matching prior solutions are prepended as a preamble. The model never
///    has to call `solution_find` explicitly.
/// 2. **Conversation recording** — every tool call writes one row to
///    `conversation_turns` so the model can later traverse "what else
///    happened in this conversation" via `conversation_show`, and solutions
///    recorded mid-call back-link to their conversation.
///
/// Conversation-traversal tools (`conversation_*`, `solution_conversations`)
/// are themselves recorded — that's intentional, traversal calls are part of
/// the conversation too.
fn route(skill: Box<dyn Skill>) -> ToolRoute<Lodestone> {
    let tool_name: &'static str = skill.name();
    let tool = Tool::new(
        tool_name.to_string(),
        skill.description().to_string(),
        skill.schema(),
    );
    ToolRoute::new_dyn(tool, move |ctx: ToolCallContext<'_, Lodestone>| {
        let server = ctx.service;
        let args = ctx.arguments.unwrap_or_default();
        let trigger = intent_trigger(tool_name, &args);
        // Runtime kill-switch from the dashboard's Tools settings drawer.
        // Reject before we run the skill body — also short-circuits the
        // auto-recall / conversation-recording side effects so a
        // disabled tool leaves no trace.
        if server
            .runtime_disabled_tools
            .lock()
            .unwrap()
            .contains(tool_name)
        {
            let err = rmcp::ErrorData::invalid_request(
                format!("tool '{tool_name}' is disabled at runtime via the dashboard settings"),
                None,
            );
            return Box::pin(async move { Err(err) });
        }
        let sctx = SkillCtx { server, args };
        let fut = skill.call(sctx);
        Box::pin(async move {
            let mut result = fut.await?;
            if server.memory.enabled() {
                let cfg = server.memory.config();
                if server.memory.auto_recall_enabled() {
                    if let Some(q) = trigger.as_deref() {
                        let mut hits = server
                            .memory
                            .auto_recall(&server.http, q, cfg.recall_max_hits.max(1))
                            .await;
                        // Auto-aliasing: when the top hit fired only via the
                        // semantic path AND the query carries enough
                        // structure, attach the query to that solution as a
                        // new phrasing. Future token-shaped recall finds it
                        // without re-running embeddings, and the recall
                        // layer's hit rate grows with use rather than
                        // ossifying around whatever wording the model
                        // happened to use first.
                        if cfg.auto_alias_on_semantic_recall
                            && !cfg.embedding_endpoint.trim().is_empty()
                            && !hits.is_empty()
                            && hits[0].was_semantic_only(cfg.recall_threshold)
                            && server.memory.query_concept_token_count(q)
                                >= cfg.auto_alias_min_query_tokens
                        {
                            let top_id = hits[0].id.clone();
                            let attached = server
                                .memory
                                .auto_attach_phrasing(&server.http, &top_id, q)
                                .await;
                            if attached {
                                hits[0].auto_attached_as_phrasing = true;
                            }
                        }
                        let mut preamble_text = if !hits.is_empty() {
                            recall_preamble(&hits)
                        } else {
                            String::new()
                        };
                        // Memo half of the preamble — small LIKE search
                        // against the memo store. Cap at 3 hits so the
                        // preamble doesn't dwarf the actual tool
                        // response. Gated by the same auto_recall
                        // master switch plus the per-feature
                        // auto_recall_facts toggle.
                        if cfg.auto_recall_facts {
                            let memo_block = server
                                .memory
                                .memo_recall_block(q, cfg.recall_max_hits.clamp(1, 3))
                                .await;
                            if !memo_block.is_empty() {
                                if !preamble_text.is_empty() {
                                    preamble_text.push('\n');
                                }
                                preamble_text.push_str(&memo_block);
                            }
                        }
                        if !preamble_text.is_empty() {
                            let preamble = rmcp::model::Content::text(preamble_text);
                            result.content.insert(0, preamble);
                        }
                    }
                }
                // Record one conversation turn per tool call. Skip when
                // `record_conversations` is off; the helper also drops
                // query-less calls when `record_only_query_calls` is on.
                if server.memory.record_conversations_enabled() {
                    if let Some(conv_id) = server.memory.current_conversation_id().await {
                        let excerpt = result
                            .content
                            .iter()
                            .find_map(|c| match &c.raw {
                                rmcp::model::RawContent::Text(t) => Some(t.text.as_str()),
                                _ => None,
                            })
                            .unwrap_or("");
                        server
                            .memory
                            .record_turn(&conv_id, tool_name, trigger.as_deref(), excerpt)
                            .await;
                    }
                }
            }
            Ok(result)
        })
    })
}

/// Every fixed skill as a boxed object (excludes the dynamic per-provider tools).
fn all_skills() -> Vec<Box<dyn Skill>> {
    let mut skills: Vec<Box<dyn Skill>> = Vec::new();
    skills.extend(search::skills());
    skills.extend(retrieve::skills());
    skills.extend(archive::skills());
    skills.extend(rfc::skills());
    skills.extend(standards::skills());
    skills.extend(arxiv::skills());
    skills.extend(huggingface::skills());
    skills.extend(wikipedia::skills());
    skills.extend(news::skills());
    skills.extend(pubmed::skills());
    skills.extend(openaccess::skills());
    skills.extend(kernel::skills());
    skills.extend(github::skills());
    skills.extend(oci::skills());
    skills.extend(artifacthub::skills());
    skills.extend(docker::skills());
    skills.extend(kubernetes::skills());
    skills.extend(filesystem::skills());
    skills.extend(ffmpeg::skills());
    skills.extend(fcc::skills());
    skills.extend(chart::skills());
    skills.extend(image::skills());
    skills.extend(html::skills());
    skills.extend(browser_session::skills());
    skills.extend(spreadsheet::skills());
    skills.extend(shell::skills());
    skills.extend(git::skills());
    skills.extend(sysinfo::skills());
    skills.extend(databases::skills());
    skills.extend(store::skills());
    skills.extend(tasks::skills());
    skills.extend(memory::skills());
    skills.extend(signal::skills());
    skills.extend(wave::skills());
    skills.extend(binary::skills());
    skills.extend(pcap::skills());
    skills.extend(disasm::skills());
    skills.extend(notebook::skills());
    skills.extend(python::skills());
    skills.extend(systemd::skills());
    skills.extend(astro::skills());
    skills.extend(radio::skills());
    skills.extend(osm::skills());
    skills.extend(grid::skills());
    skills.extend(eia::skills());
    skills.extend(noaa::skills());
    skills.extend(peeringdb::skills());
    skills.extend(weather::skills());
    skills.extend(datetime::skills());
    skills.extend(translate::skills());
    skills.extend(data::skills());
    skills.extend(regex::skills());
    skills.extend(arithmetic::skills());
    skills.extend(algebra::skills());
    skills.extend(geometry::skills());
    skills.extend(trigonometry::skills());
    skills.extend(physics::skills());
    skills.extend(finance::skills());
    skills.extend(forecast::skills());
    skills.extend(units::skills());
    skills.extend(nasa::skills());
    skills.extend(stocks::skills());
    skills.extend(yahoo::skills());
    skills.extend(satellite::skills());
    skills.extend(serial::skills());
    skills.extend(sdr::skills());
    skills.extend(printer::skills());
    skills.extend(meta::skills());
    skills
}

/// Every skill, as routes ready to add to the router. Includes the auto-generated
/// per-provider `<kind>_<id>` tools (built from the registry).
pub fn all_routes(registry: &crate::provider::Registry) -> Vec<ToolRoute<Lodestone>> {
    let mut routes: Vec<ToolRoute<Lodestone>> = all_skills().into_iter().map(route).collect();
    routes.extend(search::provider_routes(registry));
    routes
}

/// The flat list of every fixed skill's tool name (no per-provider tools).
/// The `features` skill walks this to count "how many tools in this family
/// are visible right now?" without dragging the full ToolRouter API in.
pub fn registered_tool_names() -> Vec<String> {
    all_skills()
        .into_iter()
        .map(|s| s.name().to_string())
        .collect()
}

/// Tool names the current config gates off. A local-system family is hidden in
/// full only when it's *disabled*; its destructive actions stay exposed and are
/// gated at **call time** by the confirmation [`guard`] (so any client gets the
/// "confirm / trust / cancel" prompt, with `allow_destructive` as pre-authorization).
/// Each family declares its own `TOOL_NAMES`, so `main.rs` hardcodes none.
pub fn disabled_by_config(cfg: &crate::config::Config) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut hide_if_off = |enabled: bool, all: &[&str]| {
        if !enabled {
            out.extend(all.iter().map(|s| s.to_string()));
        }
    };
    hide_if_off(cfg.docker.enabled, docker::TOOL_NAMES);
    hide_if_off(cfg.kubernetes.enabled, kubernetes::TOOL_NAMES);
    hide_if_off(cfg.filesystem.enabled, filesystem::TOOL_NAMES);
    hide_if_off(cfg.shell.enabled, shell::TOOL_NAMES);
    hide_if_off(cfg.git.enabled, git::TOOL_NAMES);
    hide_if_off(cfg.sysinfo.enabled, sysinfo::TOOL_NAMES);
    // FFmpeg conversion — off by default (needs a local ffmpeg).
    hide_if_off(cfg.ffmpeg.enabled, ffmpeg::TOOL_NAMES);
    hide_if_off(cfg.fcc.enabled, fcc::TOOL_NAMES);
    hide_if_off(cfg.chart.enabled, chart::TOOL_NAMES);
    hide_if_off(cfg.image.enabled, image::TOOL_NAMES);
    hide_if_off(cfg.html.enabled, html::TOOL_NAMES);
    // Spreadsheet read/query/write — off by default (file I/O).
    hide_if_off(cfg.spreadsheet.enabled, spreadsheet::TOOL_NAMES);
    // Database tools (ad-hoc connections, no preconfiguration) — off by default.
    hide_if_off(cfg.databases.enabled, databases::TOOL_NAMES);
    // File-store tools are gated by [store] (cache_status stays always-on).
    hide_if_off(cfg.store.enabled, store::TOOL_NAMES);
    // Serial / printer / SDR hardware skills — off by default.
    hide_if_off(cfg.serial.enabled, serial::TOOL_NAMES);
    hide_if_off(cfg.printer.enabled, printer::TOOL_NAMES);
    hide_if_off(cfg.sdr.enabled, sdr::TOOL_NAMES);
    // Background tasks — off by default.
    hide_if_off(cfg.tasks.enabled, tasks::TOOL_NAMES);
    // Memory & solution-history skills — on by default; gateable.
    hide_if_off(cfg.memory.enabled, memory::TOOL_NAMES);
    hide_if_off(cfg.signal.enabled, signal::TOOL_NAMES);
    hide_if_off(cfg.wave.enabled, wave::TOOL_NAMES);
    hide_if_off(cfg.binary.enabled, binary::TOOL_NAMES);
    hide_if_off(cfg.pcap.enabled, pcap::TOOL_NAMES);
    hide_if_off(cfg.disasm.enabled, disasm::TOOL_NAMES);
    hide_if_off(cfg.notebook.enabled, notebook::TOOL_NAMES);
    hide_if_off(cfg.python.enabled, python::TOOL_NAMES);
    hide_if_off(cfg.systemd.enabled, systemd::TOOL_NAMES);
    hide_if_off(cfg.astro.enabled, astro::TOOL_NAMES);
    hide_if_off(cfg.radio.enabled, radio::TOOL_NAMES);
    // Stock quotes — on by default, but gateable. Yahoo Finance shares the gate.
    hide_if_off(cfg.stocks.enabled, stocks::TOOL_NAMES);
    hide_if_off(cfg.stocks.enabled, yahoo::TOOL_NAMES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(map: serde_json::Value) -> JsonObject {
        map.as_object().unwrap().clone()
    }

    #[test]
    fn intent_trigger_fires_on_query_carrying_tools() {
        let a = args(json!({"query": "deploy lodestone behind nginx"}));
        for tool in [
            "web_search",
            "arxiv_search",
            "osm_overpass",
            "hf_model_search",
        ] {
            assert_eq!(
                intent_trigger(tool, &a).as_deref(),
                Some("deploy lodestone behind nginx"),
                "tool {tool} should trigger"
            );
        }
    }

    #[test]
    fn intent_trigger_skips_self_and_admin() {
        let a = args(json!({"query": "anything"}));
        for tool in [
            "solution_find",
            "solution_record",
            "memory_save",
            "memory_search",
            "synonym_add",
        ] {
            assert!(
                intent_trigger(tool, &a).is_none(),
                "tool {tool} must not trigger"
            );
        }
    }

    #[test]
    fn intent_trigger_returns_none_when_no_query_field() {
        let no_query = args(json!({"path": "/some/file", "max": 10}));
        for tool in ["fs_read", "weather_forecast", "docker_ps"] {
            assert!(intent_trigger(tool, &no_query).is_none(), "{tool}");
        }
    }

    #[test]
    fn intent_trigger_trims_and_drops_empties() {
        let blank = args(json!({"query": "   "}));
        assert!(intent_trigger("web_search", &blank).is_none());
        let padded = args(json!({"query": "  hello world  "}));
        assert_eq!(
            intent_trigger("web_search", &padded).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn recall_preamble_contains_id_score_and_navigation_hint() {
        let hits = vec![memory::RecallHit {
            id: "sol-3".into(),
            problem: "Deploy lodestone behind nginx with TLS".into(),
            score: 78.0,
            token_score: 78.0,
            semantic_score: 0.0,
            summary: "Use a reverse proxy with Let's Encrypt".into(),
            links: vec![],
            superseded_by_head: None,
            auto_attached_as_phrasing: false,
        }];
        let s = recall_preamble(&hits);
        assert!(s.starts_with("💡"));
        assert!(s.contains("1 prior solution"));
        assert!(s.contains("sol-3"));
        assert!(s.contains("78.0"));
        assert!(s.contains("Deploy lodestone behind nginx with TLS"));
        assert!(s.contains("Let's Encrypt"));
        // Without links we point to solution_show.
        assert!(s.contains("solution_show id=\"sol-3\""));
        // Must label as advisory so the model doesn't treat it as authoritative.
        assert!(s.contains("advisory"));
        // No supersession data, no warning.
        assert!(!s.contains("superseded"));
    }

    /// When the recalled hit has typed links, the preamble must surface them
    /// so the model sees the subgraph, not just the isolated solution. This
    /// is the difference between "explicit" and "intrinsic" relationship
    /// awareness.
    #[test]
    fn recall_preamble_surfaces_subgraph_when_links_exist() {
        let hits = vec![memory::RecallHit {
            id: "sol-3".into(),
            problem: "Deploy lodestone behind nginx with TLS".into(),
            score: 78.0,
            token_score: 78.0,
            semantic_score: 0.0,
            summary: "Use a reverse proxy with ACME".into(),
            links: vec![
                ("supersedes".into(), "sol-1".into()),
                ("depends-on".into(), "sol-7".into()),
                ("related-to".into(), "sol-9".into()),
            ],
            superseded_by_head: None,
            auto_attached_as_phrasing: false,
        }];
        let s = recall_preamble(&hits);
        assert!(s.contains("─supersedes→ sol-1"));
        assert!(s.contains("─depends-on→ sol-7"));
        assert!(s.contains("─related-to→ sol-9"));
        // With links we direct the model toward graph walkers, not just show.
        assert!(s.contains("solution_graph id=\"sol-3\""));
        assert!(s.contains("solution_related id=\"sol-3\""));
    }

    /// When the auto-recall walk found a head for a `superseded-by` chain
    /// that's not the hit itself, the preamble must point the model at that
    /// head loudly — surfacing the obsolete hit without the warning would
    /// silently steer the model into stale prior work.
    #[test]
    fn recall_preamble_warns_when_hit_has_been_superseded() {
        let hits = vec![memory::RecallHit {
            id: "sol-3".into(),
            problem: "Deploy lodestone behind nginx with TLS".into(),
            score: 78.0,
            token_score: 78.0,
            semantic_score: 0.0,
            summary: "Old approach using certbot".into(),
            links: vec![("superseded-by".into(), "sol-5".into())],
            superseded_by_head: Some("sol-9".into()),
            auto_attached_as_phrasing: false,
        }];
        let s = recall_preamble(&hits);
        assert!(s.contains("⚠ superseded"));
        assert!(s.contains("sol-9"));
        assert!(s.contains("prefer it"));
    }

    /// Edge case: head == hit. This happens when the head walk lands back on
    /// the starting node (shouldn't happen in practice given the visited set,
    /// but we still defend against it). No warning should fire.
    #[test]
    fn recall_preamble_does_not_warn_when_head_equals_hit() {
        let hits = vec![memory::RecallHit {
            id: "sol-3".into(),
            problem: "p".into(),
            score: 50.0,
            token_score: 50.0,
            semantic_score: 0.0,
            summary: "".into(),
            links: vec![],
            superseded_by_head: Some("sol-3".into()),
            auto_attached_as_phrasing: false,
        }];
        let s = recall_preamble(&hits);
        assert!(!s.contains("⚠"));
        assert!(!s.contains("superseded"));
    }
}
