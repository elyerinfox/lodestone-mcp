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

pub mod archive;
pub mod artifacthub;
pub mod arxiv;
pub mod data;
pub mod datetime;
pub mod docker;
pub mod filesystem;
pub mod git;
pub mod github;
pub mod huggingface;
pub mod kernel;
pub mod kubernetes;
pub mod math;
pub mod meta;
pub mod oci;
pub mod regex;
pub mod retrieve;
pub mod rfc;
pub mod search;
pub mod shell;
pub mod standards;
pub mod translate;
pub mod units;
pub mod wikipedia;

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
    skills.extend(kernel::skills());
    skills.extend(github::skills());
    skills.extend(oci::skills());
    skills.extend(artifacthub::skills());
    skills.extend(docker::skills());
    skills.extend(kubernetes::skills());
    skills.extend(filesystem::skills());
    skills.extend(shell::skills());
    skills.extend(git::skills());
    skills.extend(datetime::skills());
    skills.extend(translate::skills());
    skills.extend(data::skills());
    skills.extend(regex::skills());
    skills.extend(math::skills());
    skills.extend(units::skills());
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

/// Tool names the current config gates off — each gated family declares its own
/// tool/destructive names (`docker::TOOL_NAMES`, …), so `main.rs` hardcodes none.
pub fn disabled_by_config(cfg: &crate::config::Config) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut gate = |enabled: bool, allow_destructive: bool, all: &[&str], destructive: &[&str]| {
        if !enabled {
            out.extend(all.iter().map(|s| s.to_string()));
        } else if !allow_destructive {
            out.extend(destructive.iter().map(|s| s.to_string()));
        }
    };
    gate(
        cfg.docker.enabled,
        cfg.docker.allow_destructive,
        docker::TOOL_NAMES,
        docker::DESTRUCTIVE_NAMES,
    );
    gate(
        cfg.kubernetes.enabled,
        cfg.kubernetes.allow_destructive,
        kubernetes::TOOL_NAMES,
        kubernetes::DESTRUCTIVE_NAMES,
    );
    gate(
        cfg.filesystem.enabled,
        cfg.filesystem.allow_destructive,
        filesystem::TOOL_NAMES,
        filesystem::DESTRUCTIVE_NAMES,
    );
    // Shell is gated solely by `enabled` (no destructive subset; allowlist policy
    // is enforced at call time).
    gate(
        cfg.shell.enabled,
        true,
        shell::TOOL_NAMES,
        shell::DESTRUCTIVE_NAMES,
    );
    // Git: gated by `enabled`; destructive subcommands are checked at call time.
    gate(
        cfg.git.enabled,
        true,
        git::TOOL_NAMES,
        git::DESTRUCTIVE_NAMES,
    );
    out
}
