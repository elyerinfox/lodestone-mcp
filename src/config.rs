//! Runtime configuration: which providers to use (and in what priority) per
//! kind, the bind address, and Q&A defaults.
//!
//! Precedence (lowest to highest): built-in defaults < `lodestone.toml`
//! (or `$LODESTONE_CONFIG`) < environment variables.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `host:port` the HTTP server binds to.
    pub bind: String,
    /// Optional bearer token. When non-empty, every request to `/mcp` must send
    /// `Authorization: Bearer <token>`; otherwise it's rejected with 401. Empty
    /// (default) leaves the endpoint open. `/health` is never authenticated.
    /// Recommended when binding to `0.0.0.0` (containers/LAN).
    pub auth_token: String,
    pub tools: Tools,
    pub providers: Providers,
    pub search: Search,
    pub code: CodeSearch,
    pub stackexchange: StackExchange,
    pub google: Google,
    pub github: Github,
    pub searxng: Searxng,
    pub brave: Brave,
    pub google_cse: GoogleCse,
    pub retrieval: Retrieval,
    pub cache: Cache,
    pub store: Store,
    pub network: Network,
    pub galaxy: Galaxy,
    pub docker: Docker,
    pub kubernetes: Kubernetes,
    pub filesystem: Filesystem,
    pub shell: Shell,
    pub git: Git,
    pub sysinfo: Sysinfo,
    pub ffmpeg: Ffmpeg,
    pub fcc: Fcc,
    pub spreadsheet: Spreadsheet,
    pub sdr: Sdr,
    pub tasks: Tasks,
    pub serial: Serial,
    pub printer: Printer,
    pub nasa: Nasa,
    /// EIA Open Data API key (free at eia.gov/opendata/register.php).
    pub eia: Eia,
    pub stocks: Stocks,
    /// User-defined self-hosted forges, keyed by provider id. Each entry becomes
    /// a keyless code provider (and a `code_<id>` tool) once its id is listed in
    /// `[providers].code`. Example: `[forges.myhost] kind = "gitea", domain =
    /// "git.example.com"`.
    pub forges: HashMap<String, ForgeInstance>,
    /// User-defined documentation sites, keyed by provider id. Each entry becomes
    /// a keyless `docs` provider (and a `docs_<id>` tool) once its id is listed in
    /// `[providers].docs`. Example: `[docsites.mydocs] domain = "docs.example.com"`.
    pub docsites: HashMap<String, DocSiteInstance>,
    /// Database skills (`db_query` / `redis_command`). **No preconfiguration**: there
    /// is no stored connection — the caller passes a connection URL in each call (the
    /// user hands it to the model in conversation). Off by default.
    pub databases: Databases,
    /// Persistent memory & solution-history skills (`memory_*` / `solution_*` /
    /// `synonym_*`). **On by default.** Key/value memories and recorded solutions
    /// persist under `[memory].dir` and are recalled across sessions; recall fires
    /// intrinsically as a preamble on every query-bearing tool call.
    pub memory: Memory,
    /// Signal-processing skills (FFT, RMS, windowing). Off by default.
    pub signal: ToggleOnly,
    /// WAV file probe + decode (off by default).
    pub wave: ToggleOnly,
    /// Binary analysis (file detect, strings, entropy, hexdump, ELF/PE/Mach-O). Off by default.
    pub binary: ToggleOnly,
    /// Pcap file reader (off by default).
    pub pcap: ToggleOnly,
    /// x86/x64 disassembler (off by default).
    pub disasm: ToggleOnly,
    /// Jupyter notebook parser (off by default).
    pub notebook: ToggleOnly,
    /// Python runner (subprocess to system interpreter). Off by default; every run is guarded.
    pub python: Python,
    /// Linux systemd skills (off by default).
    pub systemd: Systemd,
    /// Astronomy skills (sun/moon/star, off by default).
    pub astro: ToggleOnly,
    /// Radio / RF link-budget skills (off by default).
    pub radio: ToggleOnly,
}

/// A skill family whose only knob is on/off (no extra parameters).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ToggleOnly {
    pub enabled: bool,
}

/// Python runner settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Python {
    pub enabled: bool,
    /// Interpreter to invoke. Default `python3` (Unix) / `python` (Windows).
    pub interpreter: String,
    /// Default per-call timeout in seconds (1–600). Default 30.
    pub timeout_secs: u64,
    /// `true` skips the per-run confirmation prompt (still cap-bound).
    pub allow_destructive: bool,
}

impl Default for Python {
    fn default() -> Self {
        Self {
            enabled: false,
            interpreter: String::new(),
            timeout_secs: 30,
            allow_destructive: false,
        }
    }
}

/// systemd skill settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Systemd {
    pub enabled: bool,
    /// `true` pre-authorizes start/stop/restart (skip the prompt).
    pub allow_destructive: bool,
}

/// Persistent memory & solution-history skills. Local on-disk JSONL store under
/// `dir`; never advertised to the constellation. Off by default. Destructive
/// `memory_forget` / `solution_forget` confirm at call time unless
/// `allow_destructive` pre-authorizes.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Memory {
    /// Expose the `memory_*`, `solution_*`, `synonym_*`, and `conversation_*`
    /// tools and arm the dispatch-wrapper that auto-prepends prior-solution
    /// recall to every query-bearing tool call. **On by default** — the layer
    /// is local (SQLite under `dir`), has no external dependencies, and the
    /// recall preamble is what gives the model the "I solved this before"
    /// surface. Set to false to silence the family entirely.
    pub enabled: bool,
    /// Directory for the SQLite store (`store.db`).
    /// Default: `.lodestone-memory` (relative to the server's working directory).
    pub dir: String,
    /// Pre-authorize destructive tools (`memory_forget` / `solution_forget` /
    /// `conversation_forget` / `conversation_prune`) — skip the per-call
    /// confirm-token handshake.
    pub allow_destructive: bool,
    /// Soft cap on each store (memories or solutions). Saves beyond it return an error.
    pub max_entries: usize,
    /// Per-value character cap (memory value or solution content). Larger inputs are rejected.
    pub max_value_chars: usize,

    // -------- Intrinsic recall ----------------------------------------------
    /// Whether the dispatch wrapper auto-prepends prior-solution recall to
    /// query-bearing tool responses. Independent from `enabled`: you can keep
    /// the tools available while silencing the preamble (e.g. quieter token
    /// budgets during long sessions). Default: true.
    pub auto_recall: bool,
    /// Minimum match score for a solution to fire intrinsic recall. The ranker
    /// is `exact canonical = 100 > exact concept = 80 > fuzzy = 20 + 40·j >
    /// substring = 15`, plus a per-tag boost of 5. Lower = chattier; higher =
    /// quieter and higher-signal. Default: 30.
    pub recall_threshold: f64,
    /// Max prior solutions shown in one recall preamble. Default: 3.
    pub recall_max_hits: usize,
    /// How many `superseded-by` hops the recall walker chases to find the head
    /// of a chain. 0 disables the warning entirely. Default: 5.
    pub superseded_walk_max_hops: usize,

    // -------- Conversation tracking ------------------------------------------
    /// Whether the dispatch wrapper records one row per tool call into
    /// `conversation_turns`. Independent from `enabled`: turning this off
    /// keeps the recall preamble but stops growing the conversation log.
    /// Default: true.
    pub record_conversations: bool,
    /// Seconds of silence (no tool calls) before the next call starts a fresh
    /// conversation id. Default: 1800 (30 minutes).
    pub conversation_idle_gap_secs: u64,
    /// Max characters of a tool's response retained in
    /// `conversation_turns.response_excerpt`. Large enough to be recognizable
    /// when traversing later, small enough that 100 turns fits on a page.
    /// Default: 240.
    pub conversation_turn_excerpt_max_chars: usize,
    /// Only record turns for tools that actually carry a free-text `query`.
    /// When true, low-signal local-system calls (fs_read, arithmetic_eval,
    /// docker_ps, …) are skipped to keep the log focused on intent. Default:
    /// false (record everything for full traversal).
    pub record_only_query_calls: bool,

    // -------- Retention / pruning -------------------------------------------
    /// Auto-delete conversations older than this many days. 0 = keep forever.
    /// Honored on startup when `prune_on_startup` is true, and by the manual
    /// `conversation_prune` tool. Default: 0.
    pub conversation_retention_days: u32,
    /// Soft cap on the number of stored conversations. When pruning, the
    /// newest `max_conversations` are kept and the rest are deleted. 0 =
    /// unlimited. Default: 0.
    pub max_conversations: usize,
    /// Run a retention sweep at startup against the rules above. Off by
    /// default so a misconfigured retention doesn't surprise-delete history
    /// on first boot; turn on once you've verified the policy in
    /// `conversation_prune dry_run=true`. Default: false.
    pub prune_on_startup: bool,

    // -------- Semantic recall (embeddings) ----------------------------------
    /// OpenAI-compatible `/v1/embeddings` endpoint to call when storing /
    /// recalling solutions. **Empty by default** → semantic recall is off and
    /// scoring falls back to the token-based path. LM Studio's local server
    /// serves this at `http://127.0.0.1:1234/v1/embeddings`. Failures (server
    /// down, model unreachable) degrade gracefully — the write still
    /// succeeds with `embedding=NULL`, and recall ignores the semantic path.
    pub embedding_endpoint: String,
    /// Embedding model to request. Default `text-embedding-nomic-embed-text-v1.5`.
    pub embedding_model: String,
    /// Cosine similarity threshold for semantic recall to fire on a solution.
    /// Conservative default — nomic-embed maps even loosely-related text into
    /// the 0.4-0.6 range, so a 0.55 floor excludes genuinely unrelated hits.
    pub embedding_threshold: f32,
    /// When the recall preamble fires *only* via the semantic path (the
    /// query's token score against the solution didn't clear
    /// `recall_threshold`, but the embedding cosine did), automatically
    /// attach the query as a new `solution_phrasings` row on the top hit so
    /// future token-shaped recall can find it without re-running embeddings.
    /// Quietly closes the "we'll only ever recall this in the original
    /// wording" loop. Default: true (only effective when embeddings are on).
    pub auto_alias_on_semantic_recall: bool,
    /// Minimum number of concept tokens the query must carry before
    /// `auto_alias_on_semantic_recall` will fire — guards against attaching
    /// noise like a single common noun ("campus") as a phrasing on the top
    /// hit it happens to match. Default: 3.
    pub auto_alias_min_query_tokens: usize,
}

impl Default for Memory {
    fn default() -> Self {
        Self {
            enabled: true,
            dir: ".lodestone-memory".to_string(),
            allow_destructive: false,
            max_entries: 10_000,
            max_value_chars: 64_000,
            auto_recall: true,
            recall_threshold: 30.0,
            recall_max_hits: 3,
            superseded_walk_max_hops: 5,
            record_conversations: true,
            conversation_idle_gap_secs: 30 * 60,
            conversation_turn_excerpt_max_chars: 240,
            record_only_query_calls: false,
            conversation_retention_days: 0,
            max_conversations: 0,
            prune_on_startup: false,
            embedding_endpoint: String::new(),
            embedding_model: "text-embedding-nomic-embed-text-v1.5".to_string(),
            embedding_threshold: 0.55,
            auto_alias_on_semantic_recall: true,
            auto_alias_min_query_tokens: 3,
        }
    }
}

/// Global settings for the database skills. Connections are always ad-hoc (passed per
/// call), so there are no stored instances/credentials.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Databases {
    /// Expose `db_query` / `redis_command`. Off by default.
    pub enabled: bool,
    /// Pre-authorize writes/DDL (SQL) and write/admin commands (Redis), skipping the
    /// per-call confirmation prompt. Off by default — writes confirm at call time.
    pub allow_destructive: bool,
}

/// A user-configured self-hosted code forge (GitLab or Gitea/Codeberg layout).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ForgeInstance {
    /// URL layout / search behavior: "gitlab", or "gitea" (also covers Codeberg
    /// and other Gitea instances). Determines how blob URLs are parsed.
    pub kind: String,
    /// Host the search is scoped to, e.g. "git.example.com" (no scheme).
    pub domain: String,
}

/// A user-configured documentation site, searched via a keyless site-scoped web
/// search (DuckDuckGo → Mojeek, render-aware), like the built-in framework docs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DocSiteInstance {
    /// Documentation host the search is scoped to, e.g. "docs.example.com" (no scheme).
    pub domain: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Tools {
    /// Allowlist of tools (skills) to expose. Empty = expose all. Names:
    /// web_search, code_search, docs_search, qa_search, fetch_page, render_page,
    /// webpage_to_pdf, read_pdf, fetch_repo_file, wayback_fetch, github_releases,
    /// github_user, github_repo, datetime, date_diff, time_convert, translate,
    /// detect_language, docker_search, docker_image, docker_tags, oci_tags,
    /// oci_manifest, artifacthub_search, rfc_get, rfc_search, standards_search,
    /// arxiv_search, arxiv_get, hf_model_search, hf_dataset_search, hf_model,
    /// wikipedia_search,
    /// wikipedia_summary, kernel_releases, json_query, json_format, yaml_to_json,
    /// json_to_yaml, regex_search, regex_replace, math_eval, math_solve,
    /// geo_distance, geo_azimuth, wave_frequency, compound_interest, loan_payment,
    /// currency_convert, convert_units, nasa_neo, nasa_mars_photos,
    /// stock_quote, sat_tle, sat_position, sat_observe, list_providers, constellation_status,
    /// constellation_peers, constellation_seeds. Serial (gated by [serial], off): serial_ports,
    /// serial_send, serial_read. Printer (gated by [printer], off): printer_list,
    /// printer_print. Local Docker
    /// daemon (gated by [docker]): docker_ps, docker_images, docker_inspect,
    /// docker_logs, docker_info, docker_pull, docker_run, docker_start,
    /// docker_build, docker_stop, docker_remove, docker_exec, docker_rmi.
    /// Kubernetes (gated by [kubernetes]):
    /// k8s_contexts, k8s_get, k8s_describe, k8s_logs, k8s_apply, k8s_scale,
    /// k8s_delete. Filesystem (gated by [filesystem], off by default): fs_read,
    /// fs_list, fs_stat, fs_find, fs_write, fs_edit, fs_mkdir, fs_delete, fs_move.
    /// Shell (gated by [shell], off by default): shell_run. Git (gated by [git]):
    /// git_run. System info (gated by [sysinfo]): system_info, system_disks,
    /// system_gpu. Databases (gated by [databases], off until one is configured):
    /// db_list, db_query, redis_command. Caching: cache_status (always on), plus
    /// store_fetch, store_get, store_list, store_purge (gated by [store]). Plus
    /// per-provider <kind>_<id> tools (e.g. docs_cratesio, docs_react, docs_kubernetes).
    pub enabled: Vec<String>,
    /// Denylist applied after `enabled`; these tools are never exposed.
    pub disabled: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Search {
    /// How providers are combined: "fallback" (first non-empty wins) or
    /// "aggregate" (query all concurrently and merge — a meta-search).
    pub strategy: String,
    /// Re-ranking method for "aggregate" results: "composite" (default, a
    /// multi-signal fusion), "reciprocal", "borda", "breadth" (consensus), or
    /// "interleave" (round-robin).
    pub ranking: String,
    /// Per-engine quality weights for the composite ranker (id -> weight, default
    /// 1.0). E.g. trust Mojeek a bit less: `[search.engine_weights] mojeek = 0.8`.
    pub engine_weights: std::collections::HashMap<String, f64>,
    /// Extra domains given an authority boost by the composite ranker, on top of
    /// a small built-in set (e.g. "docs.rs", "stackoverflow.com").
    pub trusted_domains: Vec<String>,
    /// Per-request HTTP timeout in seconds, shared by every scraping/API call.
    /// A slow source can't dominate latency past this.
    pub timeout_secs: u64,
    /// In "aggregate" mode, the maximum number of providers queried concurrently
    /// (the rest queue for a slot). Bounds the outbound-request burst so a wide
    /// `docs` fan-out doesn't trip engine rate limits. 0 = unlimited.
    pub max_concurrency: usize,
    /// Per-provider deadline (seconds): a provider that doesn't answer within this
    /// is dropped from the result set so one unresponsive/blocked source can't stall
    /// the whole search. Bounded by `timeout_secs`. 0 = no per-provider deadline.
    pub provider_timeout_secs: u64,
    /// Circuit breaker: after this many consecutive failures (timeout or transport/
    /// parse error) a provider is "tripped" and skipped for `breaker_cooldown_secs`,
    /// so a source actively blocking this egress IP fails fast instead of burning the
    /// per-provider deadline on every call. 0 = breaker disabled.
    pub breaker_threshold: u32,
    /// How long (seconds) a tripped provider stays skipped before a single probe is
    /// allowed through again. Only meaningful when `breaker_threshold > 0`.
    pub breaker_cooldown_secs: u64,
    /// Fuzzy/concept matching: also key each search by a normalized **concept
    /// signature** (lowercased, de-punctuated, stop-worded, stemmed, order-independent
    /// token set) so a differently-worded but equivalent query reuses a cached/peer
    /// result on an exact-key miss. Off by default: a bag-of-words signature is
    /// order-insensitive, so direction-sensitive phrasings (e.g. "json to yaml" vs
    /// "yaml to json") can collide. The constellation path stays consensus-gated.
    pub fuzzy_match: bool,
    /// Optional egress **proxy** URL (`http://…`, `socks5://…`, or `socks5h://…` —
    /// e.g. a local `arti` Tor SOCKS port). When a provider yields nothing or fails
    /// on the direct route, it's retried through the proxy (a different egress IP).
    /// Empty = no proxy route.
    pub proxy: String,
    /// When a provider yields nothing/fails on the plain routes (direct, then proxy),
    /// retry it through the **headless browser** (a real browser bypasses many
    /// bot-walls). Off by default — it needs a working Chrome and is heavier.
    pub render_fallback: bool,
    /// Optional per-kind overrides of `strategy`/`ranking`. Empty fields inherit
    /// the global values above, so e.g. web/code can `aggregate` while qa stays
    /// `fallback`.
    pub web: KindSearch,
    pub code: KindSearch,
    pub qa: KindSearch,
    pub docs: KindSearch,
}

/// Per-kind override of the search strategy/ranking. An empty string means
/// "inherit the global `[search]` value".
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct KindSearch {
    pub strategy: String,
    pub ranking: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct CodeSearch {
    /// Forge domains that code_search is scoped to (via `site:` on the web
    /// providers). Add e.g. "gitlab.com", "codeberg.org", "gitea.com".
    pub sites: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Google {
    /// Path to a Chrome/Chromium executable for headless rendering. Empty =
    /// auto-detect. Only consulted when a render/Google path actually runs.
    pub chrome_path: String,
    /// Pass `--no-sandbox` / `--disable-dev-shm-usage` to Chrome. Required when
    /// running Chrome as root, e.g. inside a Docker container.
    pub no_sandbox: bool,
    /// Extra command-line flags to pass to Chrome.
    pub args: Vec<String>,
    /// Max pages rendered concurrently on the shared headless browser (renders
    /// beyond this queue for a slot). Bounds memory/CPU under render-heavy load.
    pub render_concurrency: usize,
}

impl Default for Google {
    fn default() -> Self {
        Self {
            chrome_path: String::new(),
            no_sandbox: false,
            args: Vec::new(),
            render_concurrency: 4,
        }
    }
}

/// Opt-in peer-to-peer "constellation" settings. Disabled by default; when off, the
/// server behaves exactly as a standalone instance (no endpoints, no peers).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Network {
    /// Master switch for the constellation.
    pub enabled: bool,
    /// Optional separate host:port for the `/constellation/*` endpoints. Empty =
    /// share the main MCP `bind`. Set this to expose ONLY the constellation (e.g.
    /// forward it as a galaxy ingress) without publishing the MCP server itself.
    pub bind: String,
    /// Static peer base URLs, e.g. ["http://10.0.0.2:8000"].
    pub peers: Vec<String>,
    /// Auto-discover peers on the LAN via mDNS (only runs when `enabled`).
    pub mdns: bool,
    /// Optional shared secret required on `/constellation/*` (separate from `auth_token`).
    pub token: String,
    /// Port advertised to peers / via mDNS. 0 = derive from `bind`.
    pub advertise_port: u16,
    /// How often (seconds) to refresh peers' digests.
    pub sync_secs: u64,
    /// Per-peer request timeout in milliseconds.
    pub request_timeout_ms: u64,
    /// Maximum peers consulted for a single query.
    pub max_peers: usize,
    /// Maximum results accepted from any one peer (caps a single peer's influence).
    pub max_results_per_peer: usize,
    /// Peers that must corroborate a result before it's trusted without a local
    /// search (anti-poisoning; >= 2 means no single peer can carry a result).
    pub min_agreement: usize,
    /// How many hops a consult may be relayed through intermediary peers when a
    /// node can't reach a holder directly (0 = no relay; clamped to 2 max).
    pub relay_hops: u32,
    /// Stable node id. Empty = a random id generated per process.
    pub node_id: String,
    /// The **constellation** id — shared by all member nodes (distinct from the
    /// per-node `node_id`). Empty = a random id is chosen at startup; nodes that
    /// discover each other converge to the smallest id, so co-located meshes MERGE
    /// into one constellation. Set explicitly to pin a constellation's identity.
    pub id: String,
    /// Optional path to persist peer reputations across restarts (JSON). Empty
    /// disables persistence.
    pub state_file: String,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: String::new(),
            peers: Vec::new(),
            mdns: true,
            token: String::new(),
            advertise_port: 0,
            sync_secs: 30,
            request_timeout_ms: 1500,
            max_peers: 16,
            max_results_per_peer: 10,
            min_agreement: 2,
            relay_hops: 1,
            node_id: String::new(),
            id: String::new(),
            state_file: String::new(),
        }
    }
}

/// Galaxy **participation** settings for the main app. The galaxy *broker* is a
/// separate program (`lodestone-galaxy`) configured by its own env; this struct only
/// covers this constellation joining one or more brokers. The broker keeps a
/// directory of `{ constellation → public endpoint(s) }` and never proxies traffic —
/// constellations talk directly once introduced. Off by default (`servers` empty).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Galaxy {
    /// Broker base URLs this constellation registers with / queries. Empty = don't
    /// join any galaxy.
    pub servers: Vec<String>,
    /// This constellation's id in the galaxy directory. Empty = derive from the
    /// constellation node id.
    pub id: String,
    /// This constellation's **publicly-reachable** ingress base URLs that other
    /// constellations should peer with (e.g. ["http://1.2.3.4:8001"]). List several
    /// to distribute inbound load across multiple member nodes.
    pub ingress: Vec<String>,
    /// Optional shared secret for the broker's `/galaxy/*` endpoints (must match the
    /// broker's `LODESTONE_GALAXY_TOKEN`).
    pub token: String,
    /// How often (seconds) to register + pull the directory.
    pub heartbeat_secs: u64,
    /// How long to let *local* constellation discovery settle before contacting a
    /// broker (a node joins its own constellation first).
    pub join_warmup_secs: u64,
}

impl Default for Galaxy {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            id: String::new(),
            ingress: Vec::new(),
            token: String::new(),
            heartbeat_secs: 30,
            join_warmup_secs: 20,
        }
    }
}

/// Local Docker daemon control (`src/skills/docker.rs`). A local-system capability,
/// separate from the keyless web tools. On by default. Destructive actions
/// (`docker_stop`, `docker_remove`) are always exposed but require a per-call
/// confirmation step (see `skills::guard`); `allow_destructive` pre-authorizes them
/// (skips the prompt).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Docker {
    /// Expose the Docker daemon tools at all.
    pub enabled: bool,
    /// Pre-authorize the destructive Docker tools (`docker_stop`, `docker_remove`),
    /// skipping the per-call confirmation prompt. Off by default.
    pub allow_destructive: bool,
}

impl Default for Docker {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_destructive: false,
        }
    }
}

/// Kubernetes cluster interaction (`src/skills/kubernetes.rs`) via the API server
/// (reads your kubeconfig; no `kubectl`). On by default; safe writes (apply/scale)
/// are included. Destructive `k8s_delete` is always exposed but requires a per-call
/// confirmation step (see `skills::guard`); `allow_destructive` pre-authorizes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Kubernetes {
    /// Expose the Kubernetes tools at all.
    pub enabled: bool,
    /// Pre-authorize the destructive Kubernetes tool (`k8s_delete`), skipping the
    /// per-call confirmation prompt. Off by default.
    pub allow_destructive: bool,
    /// Path to a kubeconfig file. Empty = default (`$KUBECONFIG` / `~/.kube/config`)
    /// or in-cluster credentials.
    pub kubeconfig: String,
    /// Kubeconfig context to use. Empty = the file's current-context.
    pub context: String,
    /// Default namespace when a tool call doesn't specify one. Empty = "default".
    pub namespace: String,
}

impl Default for Kubernetes {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_destructive: false,
            kubeconfig: String::new(),
            context: String::new(),
            namespace: String::new(),
        }
    }
}

/// Local filesystem read/edit (`src/skills/filesystem.rs`). A powerful, dangerous
/// capability — **off by default**; the user must explicitly grant it. All paths
/// are confined to `roots` (default: the working directory). Destructive ops
/// (`fs_delete`, `fs_move`) require a per-call confirmation step (see
/// `skills::guard`); `allow_destructive` pre-authorizes them (skips the prompt).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Filesystem {
    /// Expose the filesystem tools at all. OFF by default — set to true to grant.
    pub enabled: bool,
    /// Pre-authorize the destructive tools (`fs_delete`, `fs_move`), skipping the
    /// per-call confirmation prompt. Off by default.
    pub allow_destructive: bool,
    /// Allowed base directories; every path must resolve inside one of these
    /// (symlinks resolved). Empty = the server's current working directory only.
    pub roots: Vec<String>,
}

/// Shell command execution (`src/skills/shell.rs`) — arbitrary code execution, the
/// most dangerous capability. **Off by default.** When enabled, commands are
/// restricted to the `allow` program list (executed directly, no shell, so
/// metacharacters are inert) unless `allow_unrestricted` runs anything via the
/// system shell.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Shell {
    /// Expose the `shell_run` tool at all. OFF by default — explicit grant.
    pub enabled: bool,
    /// Run ANY command via the system shell (full RCE). When false, only programs
    /// in `allow` may run, executed directly (no shell interpretation).
    pub allow_unrestricted: bool,
    /// Allowlisted program names (matched on the command's first token, by
    /// basename, case-insensitively). Empty + not unrestricted = nothing runs.
    pub allow: Vec<String>,
    /// Pre-authorize execution, skipping the per-call confirmation prompt. Off by
    /// default: because a shell command is arbitrary code, every `shell_run` is
    /// treated as destructive and confirms at call time (see `skills::guard`) unless
    /// this is set.
    pub allow_destructive: bool,
    /// Per-command timeout in seconds (the process is killed on timeout).
    pub timeout_secs: u64,
    /// Working directory for commands. Empty = the server's working directory.
    pub workdir: String,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_unrestricted: false,
            allow: Vec::new(),
            allow_destructive: false,
            timeout_secs: 30,
            workdir: String::new(),
        }
    }
}

/// Git CLI skill (`src/skills/git.rs`) — runs the local `git` binary (must be on
/// PATH). On by default; destructive subcommands (push/reset/clean/rebase/…)
/// require a per-call confirmation step (see `skills::guard`); `allow_destructive`
/// pre-authorizes them (skips the prompt).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Git {
    /// Expose the `git_run` tool.
    pub enabled: bool,
    /// Pre-authorize destructive subcommands (push, reset, clean, rebase,
    /// filter-branch, gc, prune, reflog), skipping the per-call confirmation prompt.
    /// Off by default.
    pub allow_destructive: bool,
    /// Default repository working directory. Empty = the server's working directory.
    pub repo: String,
    /// Per-command timeout in seconds (the process is killed on timeout).
    pub timeout_secs: u64,
}

impl Default for Git {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_destructive: false,
            repo: String::new(),
            timeout_secs: 60,
        }
    }
}

/// System-information skills (`src/skills/sysinfo.rs`) — read-only host/CPU/memory/
/// disk/GPU facts. On by default (read-only); set `enabled = false` to hide them.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Sysinfo {
    /// Expose the `system_*` tools.
    pub enabled: bool,
}

impl Default for Sysinfo {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// FFmpeg conversion skill (`src/skills/ffmpeg.rs`) — shells out to a local `ffmpeg`
/// / `ffprobe`. **Off by default**; input/output paths are confined to
/// `[filesystem].roots` and conversions go through the confirmation guard.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Ffmpeg {
    /// Expose `ffmpeg_convert` / `ffmpeg_probe`.
    pub enabled: bool,
}

/// FCC / amateur-radio reference skills (`src/skills/fcc.rs`). **On by
/// default** — every tool is read-only and either hits the public, keyless
/// FCC ULS API (`fcc_callsign`) or returns baked-in regulatory reference
/// data (`fcc_amateur_bands`, `fcc_radio_service`).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Fcc {
    /// Expose the `fcc_*` tools (callsign lookup + bandplan + radio-service
    /// regulatory reference).
    pub enabled: bool,
}

impl Default for Fcc {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Spreadsheet skill (`src/skills/spreadsheet.rs`) — read/query/write CSV & XLSX.
/// **Off by default**; paths are confined to `[filesystem].roots` and writes go
/// through the confirmation guard.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Spreadsheet {
    /// Expose the `sheet_*` tools.
    pub enabled: bool,
}

/// SDR skill (`src/skills/sdr.rs`) — list software-defined radios and sweep the
/// spectrum by shelling out to `rtl_power`/`rtl_test`/`hackrf_info`. **Off by
/// default** (hardware + native tools). Receive-only; no transmission.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Sdr {
    /// Expose the `sdr_*` tools.
    pub enabled: bool,
}

/// Background-tasks skill (`src/skills/tasks.rs`) — run long work (currently search)
/// off the request path and poll for results. **Off by default.**
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Tasks {
    /// Expose the `task_*` tools.
    pub enabled: bool,
}

/// Serial-port skill (`src/skills/serial.rs`) — read/write raw serial devices.
/// **Off by default** (hardware access); writes go through the confirmation guard.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Serial {
    /// Expose the `serial_*` tools. OFF by default — explicit grant.
    pub enabled: bool,
    /// Default baud rate when a call omits it.
    pub baud: u32,
    /// Default per-operation timeout in milliseconds.
    pub timeout_ms: u64,
}

impl Default for Serial {
    fn default() -> Self {
        Self {
            enabled: false,
            baud: 9600,
            timeout_ms: 1000,
        }
    }
}

/// Printer skill (`src/skills/printer.rs`) — list printers and print text via the
/// OS print system (CUPS `lp` / Windows spooler). **Off by default**; printing goes
/// through the confirmation guard.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Printer {
    /// Expose the `printer_*` tools. OFF by default — explicit grant.
    pub enabled: bool,
}

/// NASA open-data skills (`src/skills/nasa.rs`). Keyless-friendly: uses `DEMO_KEY`
/// when no key is set (low rate limit). A free key from api.nasa.gov raises the
/// limit — optional, never required, never logged/committed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Nasa {
    /// Optional api.nasa.gov key. Empty = `DEMO_KEY`. Prefer `LODESTONE_NASA_KEY`.
    pub key: String,
}

/// EIA Open Data v2 API key for the `eia_*` tools.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Eia {
    /// API key from https://www.eia.gov/opendata/register.php. Empty = `eia_*` tools error.
    pub key: String,
}

/// Stock-quote skill (`src/skills/stocks.rs`) — delayed quotes via the keyless
/// Stooq CSV endpoint. On by default; no key.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Stocks {
    /// Expose the `stock_quote` tool.
    pub enabled: bool,
}

impl Default for Stocks {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Cache {
    /// Cache search/retrieval results so repeated identical queries don't re-hit
    /// rate-limited engines or burn API quota.
    pub enabled: bool,
    /// Lifetime of each cached entry, in seconds.
    pub ttl_secs: u64,
    /// Maximum number of cached entries (in-memory backend bound).
    pub max_entries: usize,
    /// Cache backend: "memory" (default, process-local, cleared on restart) or
    /// "redis" (a shared store multiple instances point at). On redis-connect
    /// failure the server falls back to the in-memory backend.
    pub backend: String,
    /// Redis connection URL when `backend = "redis"`, e.g.
    /// "redis://127.0.0.1:6379". A URL is a credential — prefer the env var
    /// `LODESTONE_CACHE_REDIS_URL`; never logged or committed.
    pub redis_url: String,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_secs: 300,
            max_entries: 512,
            backend: "memory".to_string(),
            redis_url: String::new(),
        }
    }
}

/// On-disk file store (`src/store.rs`) for fetched bytes (repo files, PDFs, rendered
/// pages). **Off by default** — it writes to disk. The `store_*` tools manage it.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Store {
    /// Enable the on-disk file store and its `store_*` tools.
    pub enabled: bool,
    /// Directory for stored files. Empty = `./.lodestone-store`.
    pub dir: String,
    /// Entry lifetime in seconds (0 = no expiry).
    pub ttl_secs: u64,
    /// Total byte budget; the oldest entries are evicted past it (0 = unbounded).
    pub max_bytes: u64,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: String::new(),
            ttl_secs: 86_400,
            max_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Brave Search API (keyed). The `brave` web provider is active only when `key`
/// is set — it's a strictly optional enhancement, never required.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Brave {
    /// Brave Search API subscription token. Get one at
    /// <https://brave.com/search/api/>. Prefer the env var `LODESTONE_BRAVE_KEY`.
    pub key: String,
}

/// Google Programmable Search / Custom Search JSON API (keyed). The `google_cse`
/// web provider is active only when both `key` and `cx` are set.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct GoogleCse {
    /// API key (Google Cloud). Prefer `LODESTONE_GOOGLE_CSE_KEY`.
    pub key: String,
    /// Programmable Search Engine id (the `cx` parameter). Create one at
    /// <https://programmablesearchengine.google.com/>. Prefer `LODESTONE_GOOGLE_CSE_CX`.
    pub cx: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Searxng {
    /// Base URL of a SearXNG instance, e.g. "https://searx.example.com". Empty
    /// (default) disables the `searxng` provider. The instance must allow the
    /// JSON output format (`search.formats: [json]` in its settings).
    pub url: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Github {
    /// Optional GitHub token (classic or fine-grained with read access). When
    /// set, the `github` code provider uses GitHub's authenticated code-search
    /// API (GitHub no longer allows unauthenticated code search). Leave empty to
    /// rely on the keyless site-scoped web search instead.
    pub token: String,
}

/// Limits for the retrieval tools (fetch_page / render_page / wayback_fetch and
/// the answer-thread reader). Tunable so full pages aren't cut short.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Retrieval {
    /// Characters returned when a call omits `max_chars`.
    pub default_chars: usize,
    /// Hard cap on characters a retrieval tool may return.
    pub max_chars: usize,
}

impl Default for Retrieval {
    fn default() -> Self {
        Self {
            default_chars: 16_000,
            max_chars: 100_000,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Providers {
    /// Ordered web-search providers (fallback chain). Known: duckduckgo, mojeek.
    pub web: Vec<String>,
    /// Ordered code-search providers. Known: grep_app, duckduckgo, mojeek.
    pub code: Vec<String>,
    /// Ordered Q&A providers. Known: stackoverflow (alias: stackexchange).
    pub qa: Vec<String>,
    /// Ordered documentation providers. Known registries: cratesio, npm, mdn,
    /// rubygems, packagist, nuget, hex, aur, dockerhub, archlinux. Known doc sites:
    /// php, laravel, vue, react, svelte, angular, nextjs, nuxt, django, flask,
    /// fastapi, rails, spring, tailwind, bootstrap, express, symfony, astro, solid, docker,
    /// kubernetes, helm, ieee, sae, nist, kernel, ffmpeg, nvidia, intel_arc, plus
    /// any `[docsites.<id>]`.
    pub docs: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct StackExchange {
    /// Default StackExchange site when a tool call doesn't specify one. Use a
    /// short API slug (the `api_site_parameter`), NOT a URL — e.g.
    /// "stackoverflow", "serverfault", "superuser", "askubuntu", "unix".
    /// Full list: https://api.stackexchange.com/2.3/sites
    pub default_site: String,
    /// Optional API key. Not a login — it just raises the per-IP request quota
    /// (~300/day keyless → ~10k/day). Prefer the LODESTONE_STACKEXCHANGE_KEY env var.
    pub key: String,
    /// Guardrail: if non-empty, only these site slugs may be searched/read; any
    /// other requested site is rejected. Empty = allow any site. Same slug format
    /// as `default_site` (e.g. ["stackoverflow", "serverfault", "unix"]).
    pub allowed_sites: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8000".to_string(),
            auth_token: String::new(),
            tools: Tools::default(),
            providers: Providers::default(),
            search: Search::default(),
            code: CodeSearch::default(),
            stackexchange: StackExchange::default(),
            google: Google::default(),
            github: Github::default(),
            searxng: Searxng::default(),
            brave: Brave::default(),
            google_cse: GoogleCse::default(),
            retrieval: Retrieval::default(),
            cache: Cache::default(),
            store: Store::default(),
            network: Network::default(),
            galaxy: Galaxy::default(),
            docker: Docker::default(),
            kubernetes: Kubernetes::default(),
            filesystem: Filesystem::default(),
            shell: Shell::default(),
            git: Git::default(),
            sysinfo: Sysinfo::default(),
            ffmpeg: Ffmpeg::default(),
            fcc: Fcc::default(),
            spreadsheet: Spreadsheet::default(),
            sdr: Sdr::default(),
            tasks: Tasks::default(),
            serial: Serial::default(),
            printer: Printer::default(),
            nasa: Nasa::default(),
            eia: Eia::default(),
            stocks: Stocks::default(),
            forges: HashMap::new(),
            docsites: HashMap::new(),
            databases: Databases::default(),
            memory: Memory::default(),
            signal: ToggleOnly::default(),
            wave: ToggleOnly::default(),
            binary: ToggleOnly::default(),
            pcap: ToggleOnly::default(),
            disasm: ToggleOnly::default(),
            notebook: ToggleOnly::default(),
            python: Python::default(),
            systemd: Systemd::default(),
            astro: ToggleOnly::default(),
            radio: ToggleOnly::default(),
        }
    }
}

impl Default for CodeSearch {
    fn default() -> Self {
        Self {
            sites: vec!["github.com".to_string()],
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            strategy: "fallback".to_string(),
            ranking: "composite".to_string(),
            engine_weights: std::collections::HashMap::new(),
            trusted_domains: Vec::new(),
            timeout_secs: 25,
            max_concurrency: 8,
            provider_timeout_secs: 10,
            breaker_threshold: 5,
            breaker_cooldown_secs: 60,
            fuzzy_match: false,
            proxy: String::new(),
            render_fallback: false,
            web: KindSearch::default(),
            code: KindSearch::default(),
            qa: KindSearch::default(),
            docs: KindSearch::default(),
        }
    }
}

impl Default for Providers {
    fn default() -> Self {
        Self {
            web: vec!["duckduckgo".into(), "mojeek".into()],
            code: vec!["grep_app".into(), "duckduckgo".into(), "mojeek".into()],
            qa: vec!["stackoverflow".into()],
            docs: vec![
                "cratesio".into(),
                "npm".into(),
                "mdn".into(),
                "php".into(),
                "laravel".into(),
                "vue".into(),
                "react".into(),
                "svelte".into(),
                "docker".into(),
                "kubernetes".into(),
                "helm".into(),
                "ieee".into(),
                "sae".into(),
                "nist".into(),
                "kernel".into(),
                "ffmpeg".into(),
                "nvidia".into(),
                "intel_arc".into(),
                "tailwind".into(),
                "bootstrap".into(),
            ],
        }
    }
}

impl Default for StackExchange {
    fn default() -> Self {
        Self {
            default_site: "stackoverflow".to_string(),
            key: String::new(),
            allowed_sites: Vec::new(),
        }
    }
}

impl Config {
    /// Load defaults, overlay a config file if present, then overlay env vars.
    /// Load configuration by layering, lowest to highest precedence:
    /// built-in defaults < files in the config directory (`config/` /
    /// `$LODESTONE_CONFIG_DIR`, merged in sorted filename order) < a personal
    /// single file (`lodestone.toml` / `$LODESTONE_CONFIG`) < environment
    /// variables. The committed `config/` is the working baseline; `lodestone.toml`
    /// (gitignored) is for personal overrides on top of it.
    pub fn load() -> Self {
        let merged = load_layered();
        let mut cfg = match toml::Value::Table(merged).try_into::<Config>() {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::warn!(error = %e, "invalid configuration; falling back to defaults");
                Config::default()
            }
        };
        cfg.apply_env();
        cfg
    }

    fn apply_env(&mut self) {
        if let Ok(bind) = std::env::var("LODESTONE_BIND") {
            self.bind = bind;
        }
        if let Ok(token) = std::env::var("LODESTONE_AUTH_TOKEN") {
            self.auth_token = token;
        }
        if let Some(list) = env_list("LODESTONE_TOOLS_ENABLED") {
            self.tools.enabled = list;
        }
        if let Some(list) = env_list("LODESTONE_TOOLS_DISABLED") {
            self.tools.disabled = list;
        }
        if let Some(list) = env_list("LODESTONE_WEB_PROVIDERS") {
            self.providers.web = list;
        }
        if let Some(list) = env_list("LODESTONE_CODE_PROVIDERS") {
            self.providers.code = list;
        }
        if let Some(list) = env_list("LODESTONE_QA_PROVIDERS") {
            self.providers.qa = list;
        }
        if let Some(list) = env_list("LODESTONE_DOCS_PROVIDERS") {
            self.providers.docs = list;
        }
        if let Ok(site) = std::env::var("LODESTONE_STACKEXCHANGE_SITE") {
            self.stackexchange.default_site = site;
        }
        if let Ok(key) = std::env::var("LODESTONE_STACKEXCHANGE_KEY") {
            self.stackexchange.key = key;
        }
        if let Some(sites) = env_list("LODESTONE_STACKEXCHANGE_ALLOWED_SITES") {
            self.stackexchange.allowed_sites = sites;
        }
        if let Ok(strategy) = std::env::var("LODESTONE_SEARCH_STRATEGY") {
            self.search.strategy = strategy;
        }
        if let Ok(ranking) = std::env::var("LODESTONE_SEARCH_RANKING") {
            self.search.ranking = ranking;
        }
        if let Ok(secs) = std::env::var("LODESTONE_SEARCH_TIMEOUT_SECS") {
            if let Ok(n) = secs.trim().parse::<u64>() {
                self.search.timeout_secs = n;
            }
        }
        if let Ok(n) = std::env::var("LODESTONE_SEARCH_MAX_CONCURRENCY") {
            if let Ok(n) = n.trim().parse::<usize>() {
                self.search.max_concurrency = n;
            }
        }
        if let Ok(n) = std::env::var("LODESTONE_SEARCH_PROVIDER_TIMEOUT_SECS") {
            if let Ok(n) = n.trim().parse::<u64>() {
                self.search.provider_timeout_secs = n;
            }
        }
        if let Ok(n) = std::env::var("LODESTONE_SEARCH_BREAKER_THRESHOLD") {
            if let Ok(n) = n.trim().parse::<u32>() {
                self.search.breaker_threshold = n;
            }
        }
        if let Ok(n) = std::env::var("LODESTONE_SEARCH_BREAKER_COOLDOWN_SECS") {
            if let Ok(n) = n.trim().parse::<u64>() {
                self.search.breaker_cooldown_secs = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_SEARCH_FUZZY_MATCH") {
            self.search.fuzzy_match = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SEARCH_PROXY") {
            self.search.proxy = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_SEARCH_RENDER_FALLBACK") {
            self.search.render_fallback = is_truthy(&v);
        }
        if let Some(sites) = env_list("LODESTONE_CODE_SITES") {
            self.code.sites = sites;
        }
        if let Ok(path) = std::env::var("LODESTONE_CHROME_PATH") {
            self.google.chrome_path = path;
        }
        if let Ok(v) = std::env::var("LODESTONE_CHROME_NO_SANDBOX") {
            self.google.no_sandbox = is_truthy(&v);
        }
        if let Some(args) = env_list("LODESTONE_CHROME_ARGS") {
            self.google.args = args;
        }
        if let Ok(n) = std::env::var("LODESTONE_RENDER_CONCURRENCY") {
            if let Ok(n) = n.trim().parse::<usize>() {
                self.google.render_concurrency = n;
            }
        }
        if let Ok(url) = std::env::var("LODESTONE_SEARXNG_URL") {
            self.searxng.url = url;
        }
        if let Ok(key) = std::env::var("LODESTONE_BRAVE_KEY") {
            self.brave.key = key;
        }
        if let Ok(key) = std::env::var("LODESTONE_GOOGLE_CSE_KEY") {
            self.google_cse.key = key;
        }
        if let Ok(cx) = std::env::var("LODESTONE_GOOGLE_CSE_CX") {
            self.google_cse.cx = cx;
        }
        if let Ok(n) = std::env::var("LODESTONE_RETRIEVAL_DEFAULT_CHARS") {
            if let Ok(n) = n.trim().parse::<usize>() {
                self.retrieval.default_chars = n;
            }
        }
        if let Ok(n) = std::env::var("LODESTONE_RETRIEVAL_MAX_CHARS") {
            if let Ok(n) = n.trim().parse::<usize>() {
                self.retrieval.max_chars = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_CACHE_ENABLED") {
            self.cache.enabled = is_truthy(&v);
        }
        if let Ok(secs) = std::env::var("LODESTONE_CACHE_TTL_SECS") {
            if let Ok(n) = secs.trim().parse::<u64>() {
                self.cache.ttl_secs = n;
            }
        }
        if let Ok(n) = std::env::var("LODESTONE_CACHE_MAX_ENTRIES") {
            if let Ok(n) = n.trim().parse::<usize>() {
                self.cache.max_entries = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_CACHE_BACKEND") {
            self.cache.backend = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_CACHE_REDIS_URL") {
            self.cache.redis_url = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_STORE_ENABLED") {
            self.store.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_STORE_DIR") {
            self.store.dir = v;
        }
        if let Ok(n) = std::env::var("LODESTONE_STORE_TTL_SECS") {
            if let Ok(n) = n.trim().parse::<u64>() {
                self.store.ttl_secs = n;
            }
        }
        if let Ok(n) = std::env::var("LODESTONE_STORE_MAX_BYTES") {
            if let Ok(n) = n.trim().parse::<u64>() {
                self.store.max_bytes = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_NETWORK_BIND") {
            self.network.bind = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_NETWORK_ENABLED") {
            self.network.enabled = is_truthy(&v);
        }
        if let Some(peers) = env_list("LODESTONE_NETWORK_PEERS") {
            self.network.peers = peers;
        }
        if let Ok(v) = std::env::var("LODESTONE_NETWORK_MDNS") {
            self.network.mdns = is_truthy(&v);
        }
        if let Ok(t) = std::env::var("LODESTONE_NETWORK_TOKEN") {
            self.network.token = t;
        }
        if let Ok(id) = std::env::var("LODESTONE_NETWORK_ID") {
            self.network.id = id;
        }
        if let Ok(id) = std::env::var("LODESTONE_NETWORK_NODE_ID") {
            self.network.node_id = id;
        }
        if let Ok(path) = std::env::var("LODESTONE_NETWORK_STATE_FILE") {
            self.network.state_file = path;
        }
        if let Some(servers) = env_list("LODESTONE_GALAXY_SERVERS") {
            self.galaxy.servers = servers;
        }
        if let Ok(v) = std::env::var("LODESTONE_GALAXY_ID") {
            self.galaxy.id = v;
        }
        if let Some(ingress) = env_list("LODESTONE_GALAXY_INGRESS") {
            self.galaxy.ingress = ingress;
        }
        if let Ok(v) = std::env::var("LODESTONE_GALAXY_TOKEN") {
            self.galaxy.token = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_DOCKER_ENABLED") {
            self.docker.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_DOCKER_ALLOW_DESTRUCTIVE") {
            self.docker.allow_destructive = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_KUBERNETES_ENABLED") {
            self.kubernetes.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_KUBERNETES_ALLOW_DESTRUCTIVE") {
            self.kubernetes.allow_destructive = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_KUBECONFIG") {
            self.kubernetes.kubeconfig = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_KUBE_CONTEXT") {
            self.kubernetes.context = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_KUBE_NAMESPACE") {
            self.kubernetes.namespace = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_FS_ENABLED") {
            self.filesystem.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_FS_ALLOW_DESTRUCTIVE") {
            self.filesystem.allow_destructive = is_truthy(&v);
        }
        if let Some(roots) = env_list("LODESTONE_FS_ROOTS") {
            self.filesystem.roots = roots;
        }
        if let Ok(v) = std::env::var("LODESTONE_SHELL_ENABLED") {
            self.shell.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SHELL_ALLOW_DESTRUCTIVE") {
            self.shell.allow_destructive = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SHELL_ALLOW_UNRESTRICTED") {
            self.shell.allow_unrestricted = is_truthy(&v);
        }
        if let Some(allow) = env_list("LODESTONE_SHELL_ALLOW") {
            self.shell.allow = allow;
        }
        if let Ok(v) = std::env::var("LODESTONE_SHELL_WORKDIR") {
            self.shell.workdir = v;
        }
        if let Ok(n) = std::env::var("LODESTONE_SHELL_TIMEOUT_SECS") {
            if let Ok(n) = n.trim().parse::<u64>() {
                self.shell.timeout_secs = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_DATABASES_ENABLED") {
            self.databases.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_DATABASES_ALLOW_DESTRUCTIVE") {
            self.databases.allow_destructive = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_GIT_ENABLED") {
            self.git.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_GIT_ALLOW_DESTRUCTIVE") {
            self.git.allow_destructive = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_GIT_REPO") {
            self.git.repo = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_SYSINFO_ENABLED") {
            self.sysinfo.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_FFMPEG_ENABLED") {
            self.ffmpeg.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_FCC_ENABLED") {
            self.fcc.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SPREADSHEET_ENABLED") {
            self.spreadsheet.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SDR_ENABLED") {
            self.sdr.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_TASKS_ENABLED") {
            self.tasks.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_ENABLED") {
            self.memory.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_DIR") {
            if !v.trim().is_empty() {
                self.memory.dir = v;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_ALLOW_DESTRUCTIVE") {
            self.memory.allow_destructive = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_AUTO_RECALL") {
            self.memory.auto_recall = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_RECALL_THRESHOLD") {
            if let Ok(n) = v.parse::<f64>() {
                self.memory.recall_threshold = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_RECALL_MAX_HITS") {
            if let Ok(n) = v.parse::<usize>() {
                self.memory.recall_max_hits = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_SUPERSEDED_WALK_MAX_HOPS") {
            if let Ok(n) = v.parse::<usize>() {
                self.memory.superseded_walk_max_hops = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_RECORD_CONVERSATIONS") {
            self.memory.record_conversations = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_CONVERSATION_IDLE_GAP_SECS") {
            if let Ok(n) = v.parse::<u64>() {
                self.memory.conversation_idle_gap_secs = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_CONVERSATION_TURN_EXCERPT_MAX_CHARS") {
            if let Ok(n) = v.parse::<usize>() {
                self.memory.conversation_turn_excerpt_max_chars = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_RECORD_ONLY_QUERY_CALLS") {
            self.memory.record_only_query_calls = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_CONVERSATION_RETENTION_DAYS") {
            if let Ok(n) = v.parse::<u32>() {
                self.memory.conversation_retention_days = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_MAX_CONVERSATIONS") {
            if let Ok(n) = v.parse::<usize>() {
                self.memory.max_conversations = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_PRUNE_ON_STARTUP") {
            self.memory.prune_on_startup = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_EMBEDDING_ENDPOINT") {
            self.memory.embedding_endpoint = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_EMBEDDING_MODEL") {
            if !v.trim().is_empty() {
                self.memory.embedding_model = v;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_EMBEDDING_THRESHOLD") {
            if let Ok(n) = v.parse::<f32>() {
                self.memory.embedding_threshold = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_AUTO_ALIAS_ON_SEMANTIC_RECALL") {
            self.memory.auto_alias_on_semantic_recall = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_MEMORY_AUTO_ALIAS_MIN_QUERY_TOKENS") {
            if let Ok(n) = v.parse::<usize>() {
                self.memory.auto_alias_min_query_tokens = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_SIGNAL_ENABLED") {
            self.signal.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_WAVE_ENABLED") {
            self.wave.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_BINARY_ENABLED") {
            self.binary.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_PCAP_ENABLED") {
            self.pcap.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_DISASM_ENABLED") {
            self.disasm.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_NOTEBOOK_ENABLED") {
            self.notebook.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_PYTHON_ENABLED") {
            self.python.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SYSTEMD_ENABLED") {
            self.systemd.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SYSTEMD_ALLOW_DESTRUCTIVE") {
            self.systemd.allow_destructive = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_ASTRO_ENABLED") {
            self.astro.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_RADIO_ENABLED") {
            self.radio.enabled = is_truthy(&v);
        }
        if let Ok(v) = std::env::var("LODESTONE_SERIAL_ENABLED") {
            self.serial.enabled = is_truthy(&v);
        }
        if let Ok(n) = std::env::var("LODESTONE_SERIAL_BAUD") {
            if let Ok(n) = n.trim().parse::<u32>() {
                self.serial.baud = n;
            }
        }
        if let Ok(v) = std::env::var("LODESTONE_PRINTER_ENABLED") {
            self.printer.enabled = is_truthy(&v);
        }
        if let Ok(v) =
            std::env::var("LODESTONE_NASA_KEY").or_else(|_| std::env::var("NASA_API_KEY"))
        {
            self.nasa.key = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_EIA_KEY").or_else(|_| std::env::var("EIA_API_KEY"))
        {
            self.eia.key = v;
        }
        if let Ok(v) = std::env::var("LODESTONE_STOCKS_ENABLED") {
            self.stocks.enabled = is_truthy(&v);
        }
        // Accept the conventional GITHUB_TOKEN as well as our namespaced var.
        if let Ok(token) =
            std::env::var("LODESTONE_GITHUB_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN"))
        {
            self.github.token = token;
        }
    }
}

/// Deep-merge, in precedence order: every `*.toml` under the config directory
/// (recursively, sorted by path) first, then a personal single file on top.
fn load_layered() -> toml::Table {
    let mut merged = toml::Table::new();

    // 1) The committed config directory — granular, per-provider files. Walked
    //    recursively and merged in sorted path order (so `00-*.toml` precede
    //    `providers/*.toml`, etc.).
    let dir = std::env::var("LODESTONE_CONFIG_DIR").unwrap_or_else(|_| "config".into());
    let mut paths = Vec::new();
    collect_toml_files(std::path::Path::new(&dir), &mut paths);
    paths.sort();
    for path in &paths {
        if let Some(table) = read_table(path) {
            merge_tables(&mut merged, table);
        }
    }

    // 2) A personal single file (gitignored) overrides the directory baseline.
    let file = std::env::var("LODESTONE_CONFIG").unwrap_or_else(|_| "lodestone.toml".into());
    if let Some(table) = read_table(std::path::Path::new(&file)) {
        merge_tables(&mut merged, table);
    }

    merged
}

/// Recursively collect every `*.toml` file under `dir`.
fn collect_toml_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_toml_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "toml") {
            out.push(path);
        }
    }
}

fn read_table(path: &std::path::Path) -> Option<toml::Table> {
    let contents = std::fs::read_to_string(path).ok()?;
    match toml::from_str::<toml::Table>(&contents) {
        Ok(table) => {
            tracing::info!(path = %path.display(), "loaded configuration file");
            Some(table)
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "skipping invalid config file");
            None
        }
    }
}

/// Recursively merge `overlay` into `base`: nested tables are merged key-by-key;
/// any other value (scalar/array) replaces what's in `base`.
fn merge_tables(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(base_t)), toml::Value::Table(overlay_t)) => {
                merge_tables(base_t, overlay_t);
            }
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn is_truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Parse a comma-separated env var into a trimmed, non-empty list.
fn env_list(key: &str) -> Option<Vec<String>> {
    let raw = std::env::var(key).ok()?;
    let list: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() {
        None
    } else {
        Some(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(s: &str) -> toml::Table {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn merge_overrides_scalars_merges_tables_replaces_arrays() {
        let mut base = table(
            r#"
            bind = "127.0.0.1:8000"
            [search]
            strategy = "fallback"
            ranking = "reciprocal"
            [providers]
            web = ["duckduckgo"]
        "#,
        );
        let overlay = table(
            r#"
            [search]
            strategy = "aggregate"
            [providers]
            web = ["mojeek", "duckduckgo"]
        "#,
        );
        merge_tables(&mut base, overlay);

        // Nested table merged key-by-key: strategy overridden, ranking preserved.
        let search = base["search"].as_table().unwrap();
        assert_eq!(search["strategy"].as_str().unwrap(), "aggregate");
        assert_eq!(search["ranking"].as_str().unwrap(), "reciprocal");
        // Arrays are replaced wholesale, never concatenated.
        let web = base["providers"].as_table().unwrap()["web"]
            .as_array()
            .unwrap();
        assert_eq!(web.len(), 2);
        // Untouched top-level scalar is kept.
        assert_eq!(base["bind"].as_str().unwrap(), "127.0.0.1:8000");
    }

    #[test]
    fn merged_table_deserializes_with_overlay_precedence() {
        let mut base = table(
            r#"
            [search]
            strategy = "fallback"
            timeout_secs = 25
            [search.qa]
            strategy = "fallback"
        "#,
        );
        let overlay = table(
            r#"
            [search]
            strategy = "aggregate"
            timeout_secs = 8
        "#,
        );
        merge_tables(&mut base, overlay);

        let cfg: Config = toml::Value::Table(base).try_into().unwrap();
        assert_eq!(cfg.search.strategy, "aggregate"); // overlay wins
        assert_eq!(cfg.search.timeout_secs, 8);
        assert_eq!(cfg.search.qa.strategy, "fallback"); // per-kind override survives
        assert!(cfg.search.web.strategy.is_empty()); // unset → inherits at runtime
    }

    #[test]
    fn partial_table_fills_defaults() {
        // A near-empty config still yields a usable Config via serde defaults.
        let cfg: Config = toml::Value::Table(table("bind = \"0.0.0.0:9000\""))
            .try_into()
            .unwrap();
        assert_eq!(cfg.bind, "0.0.0.0:9000");
        assert_eq!(cfg.search.timeout_secs, 25);
        assert_eq!(cfg.search.strategy, "fallback");
        assert!(!cfg.providers.web.is_empty());
    }
}
