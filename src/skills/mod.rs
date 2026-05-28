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

pub mod artifacthub;
pub mod docker;
pub mod kubernetes;
pub mod oci;
pub mod translate;

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

/// Every skill, as routes ready to add to the router.
pub fn all_routes() -> Vec<ToolRoute<Lodestone>> {
    let mut skills: Vec<Box<dyn Skill>> = Vec::new();
    skills.extend(oci::skills());
    skills.extend(artifacthub::skills());
    skills.extend(docker::skills());
    skills.extend(kubernetes::skills());
    skills.extend(translate::skills());
    skills.into_iter().map(route).collect()
}
