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
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::image::{
    BuildImageOptions, CreateImageOptions, ListImagesOptions, RemoveImageOptions,
};
use bollard::Docker;
use futures::future::BoxFuture;
use futures::StreamExt;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::guard::Decision;
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

/// Run a command inside a running container and return its combined output.
pub async fn exec(name: &str, cmd: &[String]) -> Result<String> {
    let docker = connect()?;
    let exec = docker
        .create_exec(
            name,
            CreateExecOptions {
                cmd: Some(cmd.to_vec()),
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                ..Default::default()
            },
        )
        .await
        .with_context(|| format!("creating exec in '{name}'"))?;
    match docker
        .start_exec(&exec.id, None)
        .await
        .with_context(|| format!("starting exec in '{name}'"))?
    {
        StartExecResults::Attached { mut output, .. } => {
            let mut body = String::new();
            while let Some(item) = output.next().await {
                match item {
                    Ok(msg) => body.push_str(&msg.to_string()),
                    Err(e) => return Err(anyhow!("reading exec output: {e}")),
                }
            }
            if body.trim().is_empty() {
                body = "(no output)".into();
            }
            Ok(format!("$ {} (in {name})\n{body}", cmd.join(" ")))
        }
        StartExecResults::Detached => Ok(format!("Started detached exec in {name}")),
    }
}

/// Remove an image (optionally forcing removal even if containers reference it).
pub async fn rmi(name: &str, force: bool) -> Result<String> {
    let docker = connect()?;
    let deleted = docker
        .remove_image(
            name,
            Some(RemoveImageOptions {
                force,
                ..Default::default()
            }),
            None,
        )
        .await
        .with_context(|| format!("removing image '{name}'"))?;
    let mut out = format!("Removed image {name}:");
    for item in &deleted {
        let v = val(item);
        if let Some(d) = v.get("Deleted").and_then(|x| x.as_str()) {
            out.push_str(&format!("\n  deleted {}", short_id(d)));
        } else if let Some(u) = v.get("Untagged").and_then(|x| x.as_str()) {
            out.push_str(&format!("\n  untagged {u}"));
        }
    }
    Ok(out)
}

/// Build an image from a local context directory (taring it for the daemon).
pub async fn build(context: &str, dockerfile: &str, tag: &str) -> Result<String> {
    // Tar the context off the async runtime (it reads the directory tree).
    let context = context.to_string();
    let tar_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let mut ar = tar::Builder::new(Vec::new());
        ar.append_dir_all(".", &context)
            .with_context(|| format!("reading build context '{context}'"))?;
        ar.into_inner().context("finalizing build context tar")
    })
    .await
    .context("build-context task failed")??;

    let docker = connect()?;
    let opts = BuildImageOptions {
        dockerfile: dockerfile.to_string(),
        t: tag.to_string(),
        rm: true,
        ..Default::default()
    };
    let mut stream = docker.build_image(opts, None, Some(bytes::Bytes::from(tar_bytes)));
    let mut log = String::new();
    while let Some(item) = stream.next().await {
        match item {
            Ok(info) => {
                if let Some(s) = info.stream {
                    log.push_str(&s);
                }
                if let Some(err) = info.error {
                    return Err(anyhow!("build failed: {err}\n{log}"));
                }
            }
            Err(e) => return Err(anyhow!("building '{tag}': {e}")),
        }
    }
    Ok(format!("Built {tag}:\n{}", log.trim_end()))
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
struct DockerStopArgs {
    /// A container name or id.
    container: String,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerRemoveArgs {
    /// A container name or id.
    container: String,
    /// Force-remove a running container (default false).
    #[serde(default)]
    force: Option<bool>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerExecArgs {
    /// A running container's name or id.
    container: String,
    /// Command to run inside the container, e.g. `ls -la /app` (parsed like a shell
    /// line, but executed directly in the container — no host shell).
    command: String,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerRmiArgs {
    /// Image name or id to remove, e.g. `nginx:alpine` or a short id.
    image: String,
    /// Force removal even if the image is tagged multiple times or in use.
    #[serde(default)]
    force: Option<bool>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, also stop asking for this tool for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerBuildArgs {
    /// Path to the build context directory (sent to the daemon as a tar).
    context: String,
    /// Image tag to apply, e.g. `myapp:latest`.
    tag: String,
    /// Dockerfile path relative to the context. Defaults to `Dockerfile`.
    #[serde(default)]
    dockerfile: Option<String>,
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
        "Stop a running container on the LOCAL Docker daemon. Destructive: the first call returns a \
        confirmation token and does nothing — call again with confirm=<token> to proceed (or \
        confirm + trust=true to allow for the session). Accepts a container name or id."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerStopArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerStopArgs>()?;
            let summary = format!("stop container {}", args.container);
            if let Decision::Challenge(msg) = server.guard.check(
                "docker_stop",
                "docker_stop",
                server.docker.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
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
        Destructive: the first call returns a confirmation token and does nothing — call again with \
        confirm=<token> to proceed (or confirm + trust=true to allow for the session)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerRemoveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerRemoveArgs>()?;
            let force = args.force.unwrap_or(false);
            let summary = format!(
                "remove container {}{}",
                args.container,
                if force { " (force)" } else { "" }
            );
            if let Decision::Challenge(msg) = server.guard.check(
                "docker_remove",
                "docker_remove",
                server.docker.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let out = remove(&args.container, force).await.map_err(internal)?;
            Ok(text_result(out))
        })
    }
}

pub struct DockerExec;
impl Skill for DockerExec {
    fn name(&self) -> &'static str {
        "docker_exec"
    }
    fn description(&self) -> &'static str {
        "Run a command inside a running LOCAL container (like `docker exec`). Powerful — the first \
        call returns a confirmation token and does nothing; call again with confirm=<token> to \
        proceed (or confirm + trust=true to allow for the session). Returns combined stdout/stderr."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerExecArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerExecArgs>()?;
            let cmd = shell_words::split(args.command.trim())
                .map_err(|e| crate::invalid(format!("could not parse command: {e}")))?;
            if cmd.is_empty() {
                return Err(crate::invalid("empty command"));
            }
            let summary = format!("exec `{}` in {}", args.command.trim(), args.container);
            if let Decision::Challenge(msg) = server.guard.check(
                "docker_exec",
                "docker_exec",
                server.docker.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let out = exec(&args.container, &cmd).await.map_err(internal)?;
            Ok(text_result(crate::util::truncate_chars(
                &out,
                server.max_chars,
            )))
        })
    }
}

pub struct DockerRmi;
impl Skill for DockerRmi {
    fn name(&self) -> &'static str {
        "docker_rmi"
    }
    fn description(&self) -> &'static str {
        "Remove an image from the LOCAL Docker daemon. Destructive: the first call returns a \
        confirmation token and does nothing — call again with confirm=<token> to proceed (or \
        confirm + trust=true to allow for the session). Pass force=true to remove a tagged/in-use image."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerRmiArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerRmiArgs>()?;
            let force = args.force.unwrap_or(false);
            let summary = format!(
                "remove image {}{}",
                args.image,
                if force { " (force)" } else { "" }
            );
            if let Decision::Challenge(msg) = server.guard.check(
                "docker_rmi",
                "docker_rmi",
                server.docker.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let out = rmi(&args.image, force).await.map_err(internal)?;
            Ok(text_result(out))
        })
    }
}

pub struct DockerBuild;
impl Skill for DockerBuild {
    fn name(&self) -> &'static str {
        "docker_build"
    }
    fn description(&self) -> &'static str {
        "Build an image on the LOCAL Docker daemon from a context directory (tarred and sent to the \
        daemon). Provide the context path and an image tag; the Dockerfile defaults to `Dockerfile` \
        in the context. Returns the build log."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerBuildArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerBuildArgs>()?;
            let dockerfile = args.dockerfile.as_deref().unwrap_or("Dockerfile");
            let out = build(&args.context, dockerfile, &args.tag)
                .await
                .map_err(internal)?;
            Ok(text_result(crate::util::truncate_chars(
                &out,
                server.max_chars,
            )))
        })
    }
}

/// All docker daemon tool names — the gating data for this family (consumed by
/// `skills::disabled_by_config` to hide the family when `[docker].enabled` is off).
/// The destructive actions (`docker_stop`, `docker_remove`) stay exposed and are
/// gated at call time by the confirmation [`crate::skills::guard`]. Kept here, with
/// the skills, so `main.rs` hardcodes nothing.
pub const TOOL_NAMES: &[&str] = &[
    "docker_ps",
    "docker_images",
    "docker_inspect",
    "docker_logs",
    "docker_info",
    "docker_pull",
    "docker_run",
    "docker_start",
    "docker_stop",
    "docker_remove",
    "docker_exec",
    "docker_rmi",
    "docker_build",
];

/// Family metadata + host probe: does this machine have a reachable
/// Docker daemon? Tries (1) `$DOCKER_HOST` if set, (2) `/var/run/
/// docker.sock` on Unix, (3) the Windows named pipe. We don't open a
/// connection here (that's async + risks a hang on a stuck daemon);
/// existence is signal enough.
pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "docker"
    }
    fn tools(&self) -> &'static [&'static str] {
        TOOL_NAMES
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::SkillCapability;
        if let Ok(h) = std::env::var("DOCKER_HOST") {
            if !h.trim().is_empty() {
                return SkillCapability::Ready;
            }
        }
        #[cfg(unix)]
        {
            if std::path::Path::new("/var/run/docker.sock").exists() {
                return SkillCapability::Ready;
            }
        }
        #[cfg(windows)]
        {
            if std::path::Path::new(r"\\.\pipe\docker_engine").exists() {
                return SkillCapability::Ready;
            }
        }
        SkillCapability::unavailable(
            "Docker daemon socket not reachable",
            "mount /var/run/docker.sock (Unix) or set DOCKER_HOST",
        )
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
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
        Box::new(DockerExec),
        Box::new(DockerRmi),
        Box::new(DockerBuild),
    ]
}
