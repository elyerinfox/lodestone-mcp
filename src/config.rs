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
    pub providers: Providers,
    pub search: Search,
    pub code: CodeSearch,
    pub stackexchange: StackExchange,
    pub google: Google,
    pub github: Github,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Search {
    /// How providers are combined: "fallback" (first non-empty wins) or
    /// "aggregate" (query all concurrently and merge — a meta-search).
    pub strategy: String,
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
    /// auto-detect. Only used when built with `--features browser`/`google`.
    pub chrome_path: String,
    /// Pass `--no-sandbox` / `--disable-dev-shm-usage` to Chrome. Required when
    /// running Chrome as root, e.g. inside a Docker container.
    pub no_sandbox: bool,
    /// Extra command-line flags to pass to Chrome.
    pub args: Vec<String>,
}

#[derive(Debug, Deserialize)]
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
    /// Default StackExchange site when a tool call doesn't specify one.
    pub default_site: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8000".to_string(),
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

impl Default for Github {
    fn default() -> Self {
        Self {
            token: String::new(),
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            strategy: "fallback".to_string(),
        }
    }
}

impl Default for Google {
    fn default() -> Self {
        Self {
            chrome_path: String::new(),
            no_sandbox: false,
            args: Vec::new(),
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
        }
    }
}

impl Config {
    /// Load defaults, overlay a config file if present, then overlay env vars.
    pub fn load() -> Self {
        let mut cfg = Self::from_file();
        cfg.apply_env();
        cfg
    }

    fn from_file() -> Self {
        let path = std::env::var("LODESTONE_CONFIG").unwrap_or_else(|_| "lodestone.toml".into());
        match std::fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<Config>(&contents) {
                Ok(cfg) => {
                    tracing::info!(path, "loaded configuration");
                    cfg
                }
                Err(e) => {
                    tracing::warn!(path, error = %e, "invalid config file; using defaults");
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    fn apply_env(&mut self) {
        if let Ok(bind) = std::env::var("LODESTONE_BIND") {
            self.bind = bind;
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
        if let Ok(strategy) = std::env::var("LODESTONE_SEARCH_STRATEGY") {
            self.search.strategy = strategy;
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
        if let Ok(token) = std::env::var("LODESTONE_GITHUB_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
            self.github.token = token;
        }
    }
}

fn is_truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
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
