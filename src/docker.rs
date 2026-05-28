//! Local Docker daemon control via the Engine API over the platform socket
//! (Windows named pipe `\\.\pipe\docker_engine`, or the unix socket — picked by
//! bollard's local defaults; honors `DOCKER_HOST`). Direct API, no `docker` CLI.
//!
//! This is a local-system capability, separate from the keyless web tools. It's
//! gated by `[docker].enabled` (on by default) and `[docker].allow_destructive`
//! (off by default — stop/remove are hidden unless enabled). Every action is its
//! own tool so an MCP host can grant permission at per-action granularity.
//!
//! bollard types are fully encapsulated here: each function returns a formatted
//! `String`, so `main.rs` never depends on bollard. Results are serialized to
//! `serde_json::Value` and read by the Docker API's PascalCase field names, which
//! is robust across bollard's typed/enum field representations.

use anyhow::{anyhow, Context, Result};
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogsOptions, RemoveContainerOptions,
    StopContainerOptions,
};
use bollard::image::{CreateImageOptions, ListImagesOptions};
use bollard::Docker;
use futures::StreamExt;
use serde_json::Value;

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

/// Compact human byte size (e.g. "36.3 MB").
fn human_size(bytes: i64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes.max(0) as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
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
        out.push_str(&format!("\n  {id}  {label}  ({})\n", human_size(size)));
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

// --- destructive (gated by [docker].allow_destructive) -----------------------

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
