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
    pub network: Network,
    pub docker: Docker,
    pub kubernetes: Kubernetes,
    /// User-defined self-hosted forges, keyed by provider id. Each entry becomes
    /// a keyless code provider (and a `code_<id>` tool) once its id is listed in
    /// `[providers].code`. Example: `[forges.myhost] kind = "gitea", domain =
    /// "git.example.com"`.
    pub forges: HashMap<String, ForgeInstance>,
    /// User-defined documentation sites, keyed by provider id. Each entry becomes
    /// a keyless `docs` provider (and a `docs_<id>` tool) once its id is listed in
    /// `[providers].docs`. Example: `[docsites.mydocs] domain = "docs.example.com"`.
    pub docsites: HashMap<String, DocSiteInstance>,
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
    /// arxiv_search, arxiv_get, hf_search, hf_model, json_query, json_format,
    /// yaml_to_json,
    /// json_to_yaml, regex_search, regex_replace, math_eval, math_solve,
    /// convert_units, list_providers, hive_status. Local Docker
    /// daemon (gated by [docker]): docker_ps, docker_images, docker_inspect,
    /// docker_logs, docker_info, docker_pull, docker_run, docker_start,
    /// docker_stop, docker_remove. Kubernetes (gated by [kubernetes]):
    /// k8s_contexts, k8s_get, k8s_describe, k8s_logs, k8s_apply, k8s_scale,
    /// k8s_delete. Plus per-provider <kind>_<id> tools (e.g. docs_cratesio,
    /// docs_react, docs_kubernetes).
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

#[derive(Debug, Default, Deserialize)]
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
}

/// Opt-in peer-to-peer "hivemind" settings. Disabled by default; when off, the
/// server behaves exactly as a standalone instance (no endpoints, no peers).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Network {
    /// Master switch for the hivemind.
    pub enabled: bool,
    /// Static peer base URLs, e.g. ["http://10.0.0.2:8000"].
    pub peers: Vec<String>,
    /// Auto-discover peers on the LAN via mDNS (only runs when `enabled`).
    pub mdns: bool,
    /// Optional shared secret required on `/hive/*` (separate from `auth_token`).
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
    /// Optional path to persist peer reputations across restarts (JSON). Empty
    /// disables persistence.
    pub state_file: String,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            enabled: false,
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
            state_file: String::new(),
        }
    }
}

/// Local Docker daemon control (`src/docker.rs`). A local-system capability,
/// separate from the keyless web tools. On by default; mutating-but-safe actions
/// (pull/run/start) are included, while destructive ones (stop/remove) are hidden
/// unless `allow_destructive` is set.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Docker {
    /// Expose the Docker daemon tools at all.
    pub enabled: bool,
    /// Also expose the destructive Docker tools (`docker_stop`, `docker_remove`).
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

/// Kubernetes cluster interaction (`src/k8s.rs`) via the API server (reads your
/// kubeconfig; no `kubectl`). On by default; safe writes (apply/scale) are
/// included, while destructive `k8s_delete` is hidden unless `allow_destructive`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Kubernetes {
    /// Expose the Kubernetes tools at all.
    pub enabled: bool,
    /// Also expose the destructive Kubernetes tools (`k8s_delete`).
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

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Cache {
    /// Cache search results in memory so repeated identical queries don't re-hit
    /// rate-limited engines or burn API quota. Cleared on restart.
    pub enabled: bool,
    /// Lifetime of each cached result list, in seconds.
    pub ttl_secs: u64,
    /// Maximum number of cached entries (memory bound).
    pub max_entries: usize,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_secs: 300,
            max_entries: 512,
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
    /// fastapi, rails, spring, tailwind, express, symfony, astro, solid, docker,
    /// kubernetes, helm, ieee, sae, nist, plus any `[docsites.<id>]`.
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
            network: Network::default(),
            docker: Docker::default(),
            kubernetes: Kubernetes::default(),
            forges: HashMap::new(),
            docsites: HashMap::new(),
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
        if let Ok(id) = std::env::var("LODESTONE_NETWORK_NODE_ID") {
            self.network.node_id = id;
        }
        if let Ok(path) = std::env::var("LODESTONE_NETWORK_STATE_FILE") {
            self.network.state_file = path;
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
