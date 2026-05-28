//! Runtime configuration: which providers to use (and in what priority) per
//! kind, the bind address, and Q&A defaults.
//!
//! Precedence (lowest to highest): built-in defaults < `lodestone.toml`
//! (or `$LODESTONE_CONFIG`) < environment variables.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `host:port` the HTTP server binds to.
    pub bind: String,
    pub tools: Tools,
    pub providers: Providers,
    pub search: Search,
    pub code: CodeSearch,
    pub stackexchange: StackExchange,
    pub google: Google,
    pub github: Github,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Tools {
    /// Allowlist of tools (skills) to expose. Empty = expose all. Names:
    /// web_search, code_search, qa_search, fetch_page, render_page,
    /// fetch_repo_file, wayback_fetch, list_providers. Plus per-provider
    /// <kind>_<id> tools (e.g. qa_stackoverflow, qa_stackoverflow_answers).
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
    /// Re-ranking method for "aggregate" results: "reciprocal" (default),
    /// "borda", "breadth" (consensus), or "interleave" (round-robin).
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

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Github {
    /// Optional GitHub token (classic or fine-grained with read access). When
    /// set, the `github` code provider uses GitHub's authenticated code-search
    /// API (GitHub no longer allows unauthenticated code search). Leave empty to
    /// rely on the keyless site-scoped web search instead.
    pub token: String,
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
            tools: Tools::default(),
            providers: Providers::default(),
            search: Search::default(),
            code: CodeSearch::default(),
            stackexchange: StackExchange::default(),
            google: Google::default(),
            github: Github::default(),
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
            ranking: "reciprocal".to_string(),
        }
    }
}

impl Default for Providers {
    fn default() -> Self {
        Self {
            web: vec!["duckduckgo".into(), "mojeek".into()],
            code: vec!["grep_app".into(), "duckduckgo".into(), "mojeek".into()],
            qa: vec!["stackoverflow".into()],
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
