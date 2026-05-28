//! Local Docker daemon skills via the Engine API over the platform socket
//! (Windows named pipe `\\.\pipe\docker_engine`, or the unix socket — picked by
//! bollard's local defaults; honors `DOCKER_HOST`). Direct API, no `docker` CLI.
//!
//! A local-system capability, separate from the keyless web tools. Gated by
//! `[docker].enabled` (on by default) and `[docker].allow_destructive` (off by
//! default — `docker_stop`/`docker_remove` are hidden unless enabled; the gating
//! lives in `main.rs::effective_disabled`). Each action is its own skill.
//!
//! bollard types are encapsulated here. Results are serialized to
//! `serde_json::Value` and read by the Docker API's PascalCase field names, which
//! is robust across bollard's typed/enum field representations.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions,
    StopContainerOptions,
};
use bollard::image::{CreateImageOptions, ListImagesOptions};
use bollard::Docker;
use futures::future::BoxFuture;
use futures::StreamExt;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::human_size;
use crate::{clamp, internal, text_result};

/// Connect to the local Docker daemon (named pipe / unix socket / `DOCKER_HOST`).
fn connect() -> Result<Docker> {
    Docker::connect_with_local_defaults()
        .context("could not connect to the Docker daemon (is Docker running?)")
}

fn val<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

fn short_id(id: &str) -> String {
    id.trim_start_matches("sha256:").chars().take(12).collect()
}

/// List containers (running, or all with `all=true`).
pub async fn ps(all: bool) -> Result<String> {
    let docker = connect()?;
    let opts = ListContainersOptions::<String> {
        all,
        ..Default::default()
    };
    let list = docker
        .list_containers(Some(opts))
        .await
        .context("listing containers")?;
    if list.is_empty() {
        return Ok(if all {
            "No containers.".into()
        } else {
            "No running containers (pass all=true to include stopped).".into()
        });
    }
    let mut out = format!("Containers ({}):\n", list.len());
    for c in &list {
        let v = val(c);
        let id = short_id(v.get("Id").and_then(|x| x.as_str()).unwrap_or(""));
        let name = v
            .get("Names")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim_start_matches('/');
        let image = v.get("Image").and_then(|x| x.as_str()).unwrap_or("");
        let state = v.get("State").and_then(|x| x.as_str()).unwrap_or("");
        let status = v.get("Status").and_then(|x| x.as_str()).unwrap_or("");
        out.push_str(&format!(
            "\n  {id}  {name}\n    image: {image}\n    {state} · {status}\n"
        ));
    }
    Ok(out)
}

/// Inspect one container (full JSON).
pub async fn inspect(name: &str) -> Result<String> {
    let docker = connect()?;
    let info = docker
        .inspect_container(name, None)
        .await
        .with_context(|| format!("inspecting container '{name}'"))?;
    let pretty = serde_json::to_string_pretty(&info).unwrap_or_default();
    Ok(format!("Container {name}:\n{pretty}"))
}

/// Read a container's logs (last `tail` lines, stdout+stderr).
pub async fn logs(name: &str, tail: usize) -> Result<String> {
    let docker = connect()?;
    let opts = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        tail: tail.to_string(),
        ..Default::default()
    };
    let mut stream = docker.logs(name, Some(opts));
    let mut body = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(msg) => body.push_str(&msg.to_string()),
            Err(e) => return Err(anyhow!("reading logs for '{name}': {e}")),
        }
    }
    if body.trim().is_empty() {
        body = "(no logs)".into();
    }
    Ok(format!("Logs for {name} (last {tail} lines):\n{body}"))
}

/// List images stored on the local daemon.
pub async fn images() -> Result<String> {
    let docker = connect()?;
    let list = docker
        .list_images(Some(ListImagesOptions::<String>::default()))
        .await
        .context("listing images")?;
    if list.is_empty() {
        return Ok("No local images.".into());
    }
    let mut out = format!("Local images ({}):\n", list.len());
    for im in &list {
        let v = val(im);
        let id = short_id(v.get("Id").and_then(|x| x.as_str()).unwrap_or(""));
        let tags: Vec<&str> = v
            .get("RepoTags")
            .and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|t| t.as_str()).collect())
            .unwrap_or_default();
        let size = v.get("Size").and_then(|x| x.as_i64()).unwrap_or(0);
        let label = if tags.is_empty() {
            "<none>".to_string()
        } else {
            tags.join(", ")
        };
        out.push_str(&format!(
            "\n  {id}  {label}  ({})\n",
            human_size(size.max(0) as u64)
        ));
    }
    Ok(out)
}

/// Daemon version + a summary of its state.
pub async fn info() -> Result<String> {
    let docker = connect()?;
    let ver = val(&docker.version().await.context("docker version")?);
    let info = val(&docker.info().await.context("docker info")?);
    let s = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let n = |v: &Value, k: &str| v.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
    Ok(format!(
        "Docker daemon:\n  version: {} (API {})\n  os/arch: {}/{}\n  containers: {} (running {})\n  images: {}\n  host: {}",
        s(&ver, "Version"),
        s(&ver, "ApiVersion"),
        s(&info, "OperatingSystem"),
        s(&info, "Architecture"),
        n(&info, "Containers"),
        n(&info, "ContainersRunning"),
        n(&info, "Images"),
        s(&info, "Name"),
    ))
}

/// Pull an image from its registry.
pub async fn pull(image: &str) -> Result<String> {
    let docker = connect()?;
    let opts = CreateImageOptions::<String> {
        from_image: image.to_string(),
        ..Default::default()
    };
    let mut stream = docker.create_image(Some(opts), None, None);
    let mut last = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(info) => {
                if let Some(s) = val(&info).get("status").and_then(|x| x.as_str()) {
                    last = s.to_string();
                }
            }
            Err(e) => return Err(anyhow!("pulling '{image}': {e}")),
        }
    }
    Ok(format!("Pulled {image} ({last})"))
}

/// Create and start a container from an image. `command` (optional) is split on
/// whitespace into the container's command.
pub async fn run(image: &str, name: Option<&str>, command: Option<&str>) -> Result<String> {
    let docker = connect()?;
    let cmd = command
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|c| c.split_whitespace().map(str::to_string).collect::<Vec<_>>());
    let config = Config::<String> {
        image: Some(image.to_string()),
        cmd,
        ..Default::default()
    };
    let options = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|n| CreateContainerOptions {
            name: n.to_string(),
            platform: None,
        });
    let created = docker
        .create_container(options, config)
        .await
        .with_context(|| format!("creating a container from '{image}'"))?;
    docker
        .start_container(
            &created.id,
            None::<bollard::container::StartContainerOptions<String>>,
        )
        .await
        .context("starting the container")?;
    Ok(format!(
        "Started container {} from {image}",
        short_id(&created.id)
    ))
}

/// Start an existing (stopped) container.
pub async fn start(name: &str) -> Result<String> {
    let docker = connect()?;
    docker
        .start_container(
            name,
            None::<bollard::container::StartContainerOptions<String>>,
        )
        .await
        .with_context(|| format!("starting container '{name}'"))?;
    Ok(format!("Started {name}"))
}

/// Stop a running container.
pub async fn stop(name: &str) -> Result<String> {
    let docker = connect()?;
    docker
        .stop_container(name, Some(StopContainerOptions { t: 10 }))
        .await
        .with_context(|| format!("stopping container '{name}'"))?;
    Ok(format!("Stopped {name}"))
}

/// Remove a container (optionally force-removing a running one).
pub async fn remove(name: &str, force: bool) -> Result<String> {
    let docker = connect()?;
    docker
        .remove_container(
            name,
            Some(RemoveContainerOptions {
                force,
                ..Default::default()
            }),
        )
        .await
        .with_context(|| format!("removing container '{name}'"))?;
    Ok(format!("Removed {name}"))
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerPsArgs {
    /// Include stopped containers, not just running ones (default false).
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerNameArgs {
    /// A container name or id.
    container: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerLogsArgs {
    /// A container name or id.
    container: String,
    /// How many trailing log lines to return. Default 200, capped 2000.
    #[serde(default)]
    tail: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerPullArgs {
    /// Image to pull, e.g. `nginx`, `nginx:1.27`, `ghcr.io/owner/image:tag`.
    image: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerRunArgs {
    /// Image to create the container from, e.g. `nginx:alpine`.
    image: String,
    /// Optional container name.
    #[serde(default)]
    name: Option<String>,
    /// Optional command to run (split on whitespace).
    #[serde(default)]
    command: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerRemoveArgs {
    /// A container name or id.
    container: String,
    /// Force-remove a running container (default false).
    #[serde(default)]
    force: Option<bool>,
}

pub struct DockerPs;
impl Skill for DockerPs {
    fn name(&self) -> &'static str {
        "docker_ps"
    }
    fn description(&self) -> &'static str {
        "List containers on the LOCAL Docker daemon (running by default; pass all=true to include \
        stopped). Talks to the daemon directly — no docker CLI. (Distinct from docker_search, which \
        searches Docker Hub.)"
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerPsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DockerPsArgs>()?;
            Ok(text_result(
                ps(args.all.unwrap_or(false)).await.map_err(internal)?,
            ))
        })
    }
}

pub struct DockerImages;
impl Skill for DockerImages {
    fn name(&self) -> &'static str {
        "docker_images"
    }
    fn description(&self) -> &'static str {
        "List images stored on the LOCAL Docker daemon (id, tags, size)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(images().await.map_err(internal)?)) })
    }
}

pub struct DockerInspect;
impl Skill for DockerInspect {
    fn name(&self) -> &'static str {
        "docker_inspect"
    }
    fn description(&self) -> &'static str {
        "Inspect a LOCAL Docker container (full JSON: config, state, mounts, networks). Accepts a \
        container name or id."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerNameArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerNameArgs>()?;
            let out = inspect(&args.container).await.map_err(internal)?;
            Ok(text_result(crate::util::truncate_chars(
                &out,
                server.max_chars,
            )))
        })
    }
}

pub struct DockerLogs;
impl Skill for DockerLogs {
    fn name(&self) -> &'static str {
        "docker_logs"
    }
    fn description(&self) -> &'static str {
        "Read a LOCAL Docker container's logs (stdout+stderr, last `tail` lines). Accepts a \
        container name or id."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerLogsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerLogsArgs>()?;
            let tail = clamp(args.tail, 200, 2000);
            let out = logs(&args.container, tail).await.map_err(internal)?;
            Ok(text_result(crate::util::truncate_chars(
                &out,
                server.max_chars,
            )))
        })
    }
}

pub struct DockerInfo;
impl Skill for DockerInfo {
    fn name(&self) -> &'static str {
        "docker_info"
    }
    fn description(&self) -> &'static str {
        "Show the LOCAL Docker daemon's version and a summary of its state (containers, images, \
        os/arch)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(info().await.map_err(internal)?)) })
    }
}

pub struct DockerPull;
impl Skill for DockerPull {
    fn name(&self) -> &'static str {
        "docker_pull"
    }
    fn description(&self) -> &'static str {
        "Pull an image onto the LOCAL Docker daemon, e.g. `nginx:1.27` or `ghcr.io/owner/image:tag`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerPullArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DockerPullArgs>()?;
            Ok(text_result(pull(&args.image).await.map_err(internal)?))
        })
    }
}

pub struct DockerRun;
impl Skill for DockerRun {
    fn name(&self) -> &'static str {
        "docker_run"
    }
    fn description(&self) -> &'static str {
        "Create and start a container on the LOCAL Docker daemon from an image, with an optional \
        name and command. Pulls the image first if needed."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerRunArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DockerRunArgs>()?;
            let out = run(&args.image, args.name.as_deref(), args.command.as_deref())
                .await
                .map_err(internal)?;
            Ok(text_result(out))
        })
    }
}

pub struct DockerStart;
impl Skill for DockerStart {
    fn name(&self) -> &'static str {
        "docker_start"
    }
    fn description(&self) -> &'static str {
        "Start an existing (stopped) container on the LOCAL Docker daemon. Accepts a container name \
        or id."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerNameArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DockerNameArgs>()?;
            Ok(text_result(start(&args.container).await.map_err(internal)?))
        })
    }
}

pub struct DockerStop;
impl Skill for DockerStop {
    fn name(&self) -> &'static str {
        "docker_stop"
    }
    fn description(&self) -> &'static str {
        "Stop a running container on the LOCAL Docker daemon. Destructive — only available when \
        [docker].allow_destructive is set. Accepts a container name or id."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerNameArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DockerNameArgs>()?;
            Ok(text_result(stop(&args.container).await.map_err(internal)?))
        })
    }
}

pub struct DockerRemove;
impl Skill for DockerRemove {
    fn name(&self) -> &'static str {
        "docker_remove"
    }
    fn description(&self) -> &'static str {
        "Remove a container from the LOCAL Docker daemon (optionally force a running one). \
        Destructive — only available when [docker].allow_destructive is set."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerRemoveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DockerRemoveArgs>()?;
            let out = remove(&args.container, args.force.unwrap_or(false))
                .await
                .map_err(internal)?;
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes (gating happens in `effective_disabled`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(DockerPs),
        Box::new(DockerImages),
        Box::new(DockerInspect),
        Box::new(DockerLogs),
        Box::new(DockerInfo),
        Box::new(DockerPull),
        Box::new(DockerRun),
        Box::new(DockerStart),
        Box::new(DockerStop),
        Box::new(DockerRemove),
    ]
}
