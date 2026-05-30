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
pub mod data;
pub mod databases;
pub mod datetime;
pub mod disasm;
pub mod docker;
pub mod eia;
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
pub mod huggingface;
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

/// Empty argument set, for skills that take no parameters.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub(crate) struct NoArgs {}

/// Turn one boxed skill into a dynamic tool route.
fn route(skill: Box<dyn Skill>) -> ToolRoute<Lodestone> {
    let tool = Tool::new(
        skill.name().to_string(),
        skill.description().to_string(),
        skill.schema(),
    );
    ToolRoute::new_dyn(tool, move |ctx: ToolCallContext<'_, Lodestone>| {
        let sctx = SkillCtx {
            server: ctx.service,
            args: ctx.arguments.unwrap_or_default(),
        };
        skill.call(sctx)
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
    // Memory & solution-history skills — off by default.
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
