//! Introspection skills — `features` (per-family on/off + knobs), `list_providers`
//! (active sources + strategy/ranking), `constellation_status` / `constellation_peers`
//! / `constellation_seeds` (the peer-to-peer mesh). All read server state.

use std::fmt::Write;
use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

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
        "constellation_status"
    }
    fn description(&self) -> &'static str {
        "Show the peer-to-peer constellation graph: this node's id and its known peers with reputation, \
        reachability, and the mesh edges they advertise. Reports that the constellation is disabled when \
        [network].enabled is false."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(ctx.server.registry.constellation_report())) })
    }
}

pub struct HivePeers;
impl Skill for HivePeers {
    fn name(&self) -> &'static str {
        "constellation_peers"
    }
    fn description(&self) -> &'static str {
        "List the constellation nodes in reach and how many hops away each is (direct peers = 1 hop; \
        nodes only reachable via a peer's advertised list are 2+). Shows each direct peer's stable \
        machine id, reputation, and reachability. Disabled-notice when [network].enabled is false."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            Ok(text_result(
                ctx.server.registry.constellation_peers_report(),
            ))
        })
    }
}

pub struct HiveSeeds;
impl Skill for HiveSeeds {
    fn name(&self) -> &'static str {
        "constellation_seeds"
    }
    fn description(&self) -> &'static str {
        "Show per-blob seed accounting for the constellation (BitTorrent-style): for each shared file/\
        page hash, how much this node has served to peers vs. fetched from them, and the served/\
        fetched ratio. Disabled-notice when [network].enabled is false."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            Ok(text_result(
                ctx.server.registry.constellation_seeds_report(),
            ))
        })
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct HiveCapabilitiesArgs {
    /// Optional capability name to filter by — `query`, `retrieval`,
    /// `blob`, or `browser`. When set, the report only shows nodes
    /// (this one + every known peer) whose published capability set
    /// includes the named capability turned ON. Answers "who can do
    /// browser work?" with a single call.
    #[serde(default)]
    cap: Option<String>,
}

pub struct HiveCapabilities;
impl Skill for HiveCapabilities {
    fn name(&self) -> &'static str {
        "constellation_capabilities"
    }
    fn description(&self) -> &'static str {
        "Show the per-feature opt-in set every constellation node currently advertises. Each row is \
         `node_id : query=ON retrieval=off blob=ON browser=off`. Use `cap=\"browser\"` (or `query` \
         / `retrieval` / `blob`) to filter to nodes that have that capability ON — handy for \
         picking a delegate before issuing a delegated request. Delegation rejects requests for a \
         capability the target peer hasn't opted in to, so this tool is the right first step."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HiveCapabilitiesArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<HiveCapabilitiesArgs>()?;
            let report = server
                .registry
                .constellation_capabilities_report(args.cap.as_deref());
            Ok(text_result(report))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FeaturesArgs {
    /// Show detail for just one family — case-insensitive match against the
    /// family key (`memory`, `constellation`, `filesystem`, `shell`, `git`,
    /// `docker`, `kubernetes`, `systemd`, `python`, `sysinfo`, `databases`,
    /// `serial`, `printer`, `sdr`, `ffmpeg`, `signal`, `wave`, `binary`,
    /// `pcap`, `disasm`, `notebook`, `store`, `tasks`, `stocks`, `nasa`,
    /// `eia`, `github`, `search`). Omit to list every family.
    #[serde(default)]
    name: Option<String>,
}

/// One family in the per-family status dump. Each family knows its TOML
/// section header, its tool-name prefix (for "is any of these tools
/// disabled?" / "how many of these are visible?"), its enabled-flag closure,
/// and a per-family `extra` formatter for richer knob info.
struct Family {
    /// Display key used in the report and the `name=` filter.
    key: &'static str,
    /// `[section]` shown next to the key.
    section: &'static str,
    /// One-line description of what the family does.
    description: &'static str,
    /// The set of tool-name prefixes / exact names the family contributes
    /// (e.g. `["fs_"]` for filesystem). Used to count visible tools.
    tool_match: &'static [&'static str],
    /// Pull the master enabled flag from the resolved config.
    enabled: fn(&crate::config::Config) -> bool,
    /// Per-family extra knob lines appended after the master flag. Each line
    /// is a small `(label, value)` pair.
    extra: fn(&crate::config::Config) -> Vec<(String, String)>,
}

fn fam_match(tools: &[String], disabled: &[String], patterns: &[&str]) -> (usize, usize) {
    let active: Vec<&String> = tools
        .iter()
        .filter(|t| {
            patterns
                .iter()
                .any(|p| t.starts_with(p) || *p == t.as_str())
        })
        .collect();
    let disabled_count = active.iter().filter(|t| disabled.contains(t)).count();
    (active.len() - disabled_count, active.len())
}

fn yn(v: bool) -> &'static str {
    if v {
        "ON"
    } else {
        "OFF"
    }
}

/// Build a `Family` entry from a 4-field signature plus a config-section
/// shorthand. Collapses the boilerplate two shapes that appear ~25 times
/// in `families()`:
///
/// 1. `family!(key, section, desc, tools, field)` — `enabled` reads
///    `c.<field>.enabled`, `extra` is empty. Used for plain on/off skills
///    (`python`, `sysinfo`, `serial`, `printer`, `chart`, …).
/// 2. `family!(key, section, desc, tools, field, destructive)` — also
///    surfaces a single `allow_destructive` knob in `extra`. Used for the
///    `filesystem` / `shell` / `git` / `docker` / `kubernetes` / `systemd`
///    / `databases` cluster, which all share the destructive-confirmation
///    pattern.
///
/// Anything more interesting (custom `enabled` predicates, multi-knob
/// `extra`, untyped config fields) writes the long-form `Family { … }`
/// literal directly — see `memory`, `network`, `nasa`, `eia`, `github`,
/// `search`, `store`.
macro_rules! family {
    ($key:literal, $section:literal, $desc:expr, $tools:expr, $field:ident $(,)?) => {
        Family {
            key: $key,
            section: $section,
            description: $desc,
            tool_match: $tools,
            enabled: |c| c.$field.enabled,
            extra: |_c| vec![],
        }
    };
    ($key:literal, $section:literal, $desc:expr, $tools:expr, $field:ident, destructive $(,)?) => {
        Family {
            key: $key,
            section: $section,
            description: $desc,
            tool_match: $tools,
            enabled: |c| c.$field.enabled,
            extra: |c| {
                vec![(
                    "allow_destructive".into(),
                    yn(c.$field.allow_destructive).into(),
                )]
            },
        }
    };
}

fn families() -> Vec<Family> {
    vec![
        Family {
            key: "memory",
            section: "[memory]",
            description: "Persistent memos, recorded solutions, conversation tracking, synonyms — \
                 plus the intrinsic recall preamble that fires on every query-bearing tool.",
            tool_match: &["memory_", "solution_", "synonym_", "conversation_"],
            enabled: |c| c.memory.enabled,
            extra: |c| {
                let m = &c.memory;
                let mut out = vec![
                    ("dir".into(), m.dir.clone()),
                    ("max_entries".into(), m.max_entries.to_string()),
                    ("max_value_chars".into(), m.max_value_chars.to_string()),
                    ("allow_destructive".into(), yn(m.allow_destructive).into()),
                    ("auto_recall".into(), yn(m.auto_recall).into()),
                    ("recall_threshold".into(), m.recall_threshold.to_string()),
                    ("recall_max_hits".into(), m.recall_max_hits.to_string()),
                    (
                        "superseded_walk_max_hops".into(),
                        m.superseded_walk_max_hops.to_string(),
                    ),
                    (
                        "record_conversations".into(),
                        yn(m.record_conversations).into(),
                    ),
                    (
                        "conversation_idle_gap_secs".into(),
                        m.conversation_idle_gap_secs.to_string(),
                    ),
                    (
                        "conversation_turn_excerpt_max_chars".into(),
                        m.conversation_turn_excerpt_max_chars.to_string(),
                    ),
                    (
                        "record_only_query_calls".into(),
                        yn(m.record_only_query_calls).into(),
                    ),
                    (
                        "conversation_retention_days".into(),
                        if m.conversation_retention_days == 0 {
                            "0 (keep forever)".into()
                        } else {
                            m.conversation_retention_days.to_string()
                        },
                    ),
                    (
                        "max_conversations".into(),
                        if m.max_conversations == 0 {
                            "0 (unlimited)".into()
                        } else {
                            m.max_conversations.to_string()
                        },
                    ),
                    ("prune_on_startup".into(), yn(m.prune_on_startup).into()),
                    (
                        "embedding_endpoint".into(),
                        if m.embedding_endpoint.trim().is_empty() {
                            "<unset — semantic recall OFF>".into()
                        } else {
                            m.embedding_endpoint.clone()
                        },
                    ),
                    ("embedding_model".into(), m.embedding_model.clone()),
                    (
                        "embedding_threshold".into(),
                        m.embedding_threshold.to_string(),
                    ),
                    (
                        "auto_alias_on_semantic_recall".into(),
                        yn(m.auto_alias_on_semantic_recall).into(),
                    ),
                    (
                        "auto_alias_min_query_tokens".into(),
                        m.auto_alias_min_query_tokens.to_string(),
                    ),
                ];
                out.retain(|(_, v)| !v.is_empty());
                out
            },
        },
        Family {
            key: "constellation",
            section: "[network]",
            description: "Peer-to-peer cache sharing across lodestone instances. \
                          Only HASHES of query keys cross the wire.",
            tool_match: &[
                "constellation_status",
                "constellation_peers",
                "constellation_seeds",
            ],
            enabled: |c| c.network.enabled,
            extra: |c| {
                vec![
                    ("peers".into(), c.network.peers.len().to_string()),
                    ("mdns".into(), yn(c.network.mdns).into()),
                    ("min_agreement".into(), c.network.min_agreement.to_string()),
                    ("sync_secs".into(), c.network.sync_secs.to_string()),
                ]
            },
        },
        family!(
            "filesystem",
            "[filesystem]",
            "Local file read/write (`fs_*`).",
            &["fs_"],
            filesystem,
            destructive
        ),
        family!(
            "shell",
            "[shell]",
            "Arbitrary subprocess execution (`shell_run`).",
            &["shell_"],
            shell,
            destructive
        ),
        family!(
            "git",
            "[git]",
            "Local git CLI passthrough (`git_run`).",
            &["git_"],
            git,
            destructive
        ),
        family!(
            "docker",
            "[docker]",
            "Local Docker daemon (`docker_*`).",
            &["docker_"],
            docker,
            destructive
        ),
        family!(
            "kubernetes",
            "[kubernetes]",
            "Kubernetes context operations (`k8s_*`).",
            &["k8s_"],
            kubernetes,
            destructive
        ),
        family!(
            "systemd",
            "[systemd]",
            "Linux systemd unit control (`systemd_*`).",
            &["systemd_"],
            systemd,
            destructive
        ),
        family!(
            "python",
            "[python]",
            "Python interpreter subprocess (`python_run`).",
            &["python_run"],
            python
        ),
        family!(
            "sysinfo",
            "[sysinfo]",
            "Host info: CPU, disks, GPU, OS release (`system_*`).",
            &["system_"],
            sysinfo
        ),
        family!(
            "databases",
            "[databases]",
            "Ad-hoc DB queries (URL per call, never preconfigured).",
            &["db_query", "redis_command"],
            databases,
            destructive
        ),
        family!(
            "serial",
            "[serial]",
            "Serial port read/write.",
            &["serial_"],
            serial
        ),
        family!(
            "printer",
            "[printer]",
            "CUPS printer listing + print.",
            &["printer_"],
            printer
        ),
        family!(
            "sdr",
            "[sdr]",
            "Software-defined radio device listing + scan.",
            &["sdr_"],
            sdr
        ),
        family!(
            "ffmpeg",
            "[ffmpeg]",
            "Media probe / convert.",
            &["ffmpeg_"],
            ffmpeg
        ),
        family!(
            "fcc",
            "[fcc]",
            "FCC callsign lookup (live ULS API) + US amateur band plan + non-amateur radio \
             services (FRS/GMRS/MURS/CB) reference.",
            &["fcc_"],
            fcc
        ),
        family!(
            "chart",
            "[chart]",
            "Chart / plot rendering — pure-Rust SVG (line, bar, scatter, histogram, pie), \
             procedural canvas, heatmaps, and interactive HTML via Chart.js / Plotly. \
             Output is responsive (SVG viewBox + HTML viewports) and embeddable.",
            &["chart_"],
            chart
        ),
        family!(
            "image",
            "[image]",
            "Image forensics + EXIF parsing — format / dimensions, full EXIF dump (incl. \
             GPS + forensic divergence flags), JPEG / PNG marker walk, embedded-thumbnail \
             extraction. Read-only, paths confined to [filesystem].roots.",
            &["image_"],
            image
        ),
        family!(
            "html",
            "[html]",
            "Render HTML / a URL in headless Chrome and capture diagnostics: every console \
             call, every uncaught JS exception with stack, every network failure, every HTTP \
             4xx/5xx response. Verifies generated UIs / `chart_interactive` HTML actually \
             runs cleanly.",
            &["html_"],
            html
        ),
        family!(
            "signal",
            "[signal]",
            "Signal-processing (FFT, RMS, windowing).",
            &["signal_"],
            signal
        ),
        family!("wave", "[wave]", "WAV file reader.", &["wave_"], wave),
        family!(
            "binary",
            "[binary]",
            "Binary analysis (ELF/PE/Mach-O probe, strings, entropy, hexdump).",
            &["binary_"],
            binary
        ),
        family!("pcap", "[pcap]", "Pcap reader.", &["pcap_"], pcap),
        family!(
            "disasm",
            "[disasm]",
            "x86/x64 disassembler.",
            &["disasm_"],
            disasm
        ),
        family!(
            "notebook",
            "[notebook]",
            "Jupyter notebook parser.",
            &["notebook_"],
            notebook
        ),
        Family {
            key: "store",
            section: "[store]",
            description: "On-disk file store (`store_*`, `cache_status`).",
            tool_match: &["store_", "cache_status"],
            enabled: |c| c.store.enabled,
            extra: |c| {
                vec![
                    ("dir".into(), c.store.dir.clone()),
                    ("max_bytes".into(), c.store.max_bytes.to_string()),
                    ("ttl_secs".into(), c.store.ttl_secs.to_string()),
                ]
            },
        },
        family!(
            "tasks",
            "[tasks]",
            "Background task queue (`task_*`).",
            &["task_"],
            tasks
        ),
        family!(
            "stocks",
            "[stocks]",
            "Stock / Yahoo finance lookups.",
            &["stock_", "yahoo_"],
            stocks
        ),
        Family {
            key: "nasa",
            section: "[nasa]",
            description: "NASA APIs (uses DEMO_KEY if no key configured; rate-limited).",
            tool_match: &["nasa_"],
            enabled: |_c| true,
            extra: |c| {
                vec![(
                    "key".into(),
                    if c.nasa.key.trim().is_empty() {
                        "<unset — uses DEMO_KEY (rate-limited)>".into()
                    } else {
                        "<set>".into()
                    },
                )]
            },
        },
        Family {
            key: "eia",
            section: "[eia]",
            description: "U.S. Energy Information Administration series.",
            tool_match: &["eia_"],
            enabled: |c| !c.eia.key.trim().is_empty(),
            extra: |c| {
                vec![(
                    "key".into(),
                    if c.eia.key.trim().is_empty() {
                        "<unset — eia_* tools are inert without a key>".into()
                    } else {
                        "<set>".into()
                    },
                )]
            },
        },
        Family {
            key: "github",
            section: "[github]",
            description: "GitHub REST (a token raises the rate limit for github_releases).",
            tool_match: &["github_"],
            enabled: |_c| true,
            extra: |c| {
                vec![(
                    "token".into(),
                    if c.github.token.trim().is_empty() {
                        "<unset — public-rate limits apply>".into()
                    } else {
                        "<set>".into()
                    },
                )]
            },
        },
        Family {
            key: "search",
            section: "[search]",
            description: "Web / code / docs / qa search across configured providers.",
            tool_match: &[
                "web_",
                "code_",
                "docs_",
                "qa_",
                "web_search",
                "code_search",
                "docs_search",
                "qa_search",
            ],
            enabled: |_c| true,
            extra: |c| {
                vec![
                    ("strategy".into(), format!("{:?}", c.search.strategy)),
                    ("ranking".into(), format!("{:?}", c.search.ranking)),
                    ("timeout_secs".into(), c.search.timeout_secs.to_string()),
                    (
                        "max_concurrency".into(),
                        c.search.max_concurrency.to_string(),
                    ),
                ]
            },
        },
    ]
}

fn render_family_block(
    fam: &Family,
    cfg: &crate::config::Config,
    all_tools: &[String],
    disabled_tools: &[String],
) -> String {
    let on = (fam.enabled)(cfg);
    let (visible, total) = fam_match(all_tools, disabled_tools, fam.tool_match);
    let mut out = String::new();
    let _ = writeln!(out, "## {} {}", fam.key, fam.section);
    let _ = writeln!(out, "  status     : {}", yn(on));
    let _ = writeln!(
        out,
        "  tools      : {visible} active / {total} total ({})",
        fam.tool_match.join(", ")
    );
    let _ = writeln!(out, "  about      : {}", fam.description);
    for (k, v) in (fam.extra)(cfg) {
        let _ = writeln!(out, "  {:<11}: {}", k, v);
    }
    out.push('\n');
    out
}

pub struct Features;
impl Skill for Features {
    fn name(&self) -> &'static str {
        "features"
    }
    fn description(&self) -> &'static str {
        "Per-family enabled/disabled status and the knob values that control each. Use this \
         before assuming a tool family is available — e.g. ask `features name=\"filesystem\"` \
         to see whether `fs_*` tools are exposed, what `allow_destructive` is set to, and how \
         many `fs_*` tools are visible right now. With no `name`, every gateable family is \
         listed (memory, constellation, filesystem, shell, git, docker, kubernetes, systemd, \
         python, sysinfo, databases, serial, printer, sdr, ffmpeg, signal, wave, binary, pcap, \
         disasm, notebook, store, tasks, stocks, nasa, eia, github, search), including live \
         counts from the memory store when memory is enabled."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FeaturesArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<FeaturesArgs>()?;
            let cfg = &server.cfg;
            let filter = args.name.as_deref().map(str::to_ascii_lowercase);

            // Build the universe of tool names we expose, so the per-family
            // active/total counts mean something. The same family-prefix logic
            // is used to render the disabled-by-config dial elsewhere.
            let mut all_tools: Vec<String> = Vec::new();
            for s in crate::skills::registered_tool_names() {
                all_tools.push(s);
            }
            let disabled = server.disabled_tools.as_slice();

            let fams = families();
            let mut out = String::from(
                "Lodestone feature inventory (resolved [memory] / [network] / per-family knobs).\n",
            );
            let _ = writeln!(
                out,
                "Bind: {}; memory enabled: {}; constellation: {}; total tools visible: {}/{}\n",
                cfg.bind,
                yn(cfg.memory.enabled),
                yn(cfg.network.enabled),
                all_tools.len() - disabled.len(),
                all_tools.len(),
            );

            let mut matched = 0usize;
            for fam in &fams {
                if let Some(f) = &filter {
                    if fam.key != f {
                        continue;
                    }
                }
                matched += 1;
                out.push_str(&render_family_block(fam, cfg, &all_tools, disabled));
            }
            if let Some(f) = &filter {
                if matched == 0 {
                    return Ok(text_result(format!(
                        "No family named \"{f}\". Try one of: {}",
                        fams.iter().map(|x| x.key).collect::<Vec<_>>().join(", ")
                    )));
                }
            }

            // Live memory counts when memory is on — let the model see how
            // much it actually has to work with, not just the gate state.
            if cfg.memory.enabled && filter.as_deref().is_none_or(|f| f == "memory") {
                let s = server.memory.stats().await;
                let _ = writeln!(out, "Live memory store contents:");
                let _ = writeln!(
                    out,
                    "  {} memos · {} solutions ({}/{} embedded) · {} revisions · {} links · {} tags",
                    s.memos,
                    s.solutions,
                    s.solutions_embedded,
                    s.solutions,
                    s.solution_revisions,
                    s.solution_links,
                    s.solution_tags,
                );
                let _ = writeln!(
                    out,
                    "  {} phrasings ({}/{} embedded) · {} synonyms · {} conversations · {} turns",
                    s.solution_phrasings,
                    s.phrasings_embedded,
                    s.solution_phrasings,
                    s.synonyms,
                    s.conversations,
                    s.conversation_turns,
                );
            }
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(Features),
        Box::new(ListProviders),
        Box::new(HiveStatus),
        Box::new(HivePeers),
        Box::new(HiveSeeds),
        Box::new(HiveCapabilities),
    ]
}
