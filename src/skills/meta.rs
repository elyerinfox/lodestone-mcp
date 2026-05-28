//! Introspection skills — `list_providers` (active sources + strategy/ranking)
//! and `hive_status` (the peer-to-peer hivemind graph). Both read server state.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::text_result;

pub struct ListProviders;
impl Skill for ListProviders {
    fn name(&self) -> &'static str {
        "list_providers"
    }
    fn description(&self) -> &'static str {
        "List the configured search providers and the order they are tried, for web, code and Q&A. \
        Useful to check which sources are active."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(ctx.server.registry.describe())) })
    }
}

pub struct HiveStatus;
impl Skill for HiveStatus {
    fn name(&self) -> &'static str {
        "hive_status"
    }
    fn description(&self) -> &'static str {
        "Show the peer-to-peer hivemind graph: this node's id and its known peers with reputation, \
        reachability, and the mesh edges they advertise. Reports that the hivemind is disabled when \
        [network].enabled is false."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(ctx.server.registry.hive_report())) })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(ListProviders), Box::new(HiveStatus)]
}
