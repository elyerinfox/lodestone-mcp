# Tools reference

Every capability lodestone exposes, grouped by purpose. Tools come in two tiers:
**general** (aggregated, everyday) and **per-provider** (target one source). Every
tool is independently gateable via `[tools]` (see
[configuration.md](configuration.md#tools-skills)). `?` marks optional arguments.

## Search

Query all configured providers of a kind, combined per `[search].strategy`. All
accept `render` to route the underlying fetch through the headless browser.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `web_search` | `query`, `max_results?`, `render?` | General web search (DuckDuckGo, Mojeek, …). |
| `code_search` | `query`, `language?`, `max_results?`, `render?` | Source-code search across the configured forges (`[code].sites`). |
| `docs_search` | `query`, `max_results?`, `render?` | Package registries (crates.io, npm, MDN) **and** framework/tooling docs (PHP, Laravel, Vue, React, Svelte, Docker, Kubernetes, Helm, …). |
| `qa_search` | `query`, `site?`, `max_results?`, `render?` | Q&A providers (StackExchange network: StackOverflow, Server Fault, …). |

## Retrieve

Fetch one known thing.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `fetch_page` | `url`, `max_chars?` | Page → readable text over plain HTTP (the default reader). |
| `render_page` | `url`, `max_chars?` | Page → readable text via a headless browser (runs JS). |
| `webpage_to_pdf` | `url`, `path?` | Render a page to a local PDF (headless browser); returns the saved path. |
| `read_pdf` | `source`, `max_chars?` | Extract a PDF's text locally — `source` is a URL or local path. |
| `fetch_repo_file` | `target`, `start_line?`, `end_line?` | A file from GitHub/GitLab/Gitea — blob/raw URL, or GitHub `owner/repo/path` (a `#L10-L40` fragment works too). |
| `wayback_fetch` | `url`, `timestamp?`, `max_chars?` | Archived snapshot from the Wayback Machine. |

## GitHub (keyless; optional `[github].token` raises the rate limit)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `github_releases` | `repo`, `max_results?`, `include_prereleases?` | A repo's releases (newest first): tag, date, notes. |
| `github_user` | `user` | A GitHub user/org profile (bio, company, repos, followers). |
| `github_repo` | `repo` | Repo metadata (stars, language, topics, license, default branch, …). |

## Containers & cloud-native (keyless)

Full detail: [containers.md](containers.md).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `docker_search` | `query`, `max_results?` | Search Docker Hub images (stars, pulls, official). |
| `docker_image` | `image` | A Docker Hub repository's metadata. |
| `docker_tags` | `image`, `max_results?` | A Docker Hub image's tags (size, date, arch). |
| `oci_tags` | `reference`, `max_results?` | List tags on **any** OCI registry (Docker Hub, GHCR, Quay, …). |
| `oci_manifest` | `reference` | Inspect a manifest: multi-arch platforms, or layers/size/config. |
| `artifacthub_search` | `query`, `kind?`, `max_results?` | Artifact Hub: Helm charts, Operators, krew, policies, Tekton. |

## Local Docker daemon

A local-system capability (direct Engine API, no CLI), gated by `[docker]` —
on by default; destructive tools hidden unless `allow_destructive` is set. Full
detail: [containers.md](containers.md#local-docker-daemon-write-access).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `docker_ps` | `all?` | read | List containers. |
| `docker_images` | — | read | List local images. |
| `docker_inspect` | `container` | read | Full container JSON. |
| `docker_logs` | `container`, `tail?` | read | Container logs. |
| `docker_info` | — | read | Daemon version + state. |
| `docker_pull` | `image` | write | Pull an image. |
| `docker_run` | `image`, `name?`, `command?` | write | Create + start a container. |
| `docker_start` | `container` | write | Start a stopped container. |
| `docker_stop` | `container` | destructive | Stop a container (opt-in). |
| `docker_remove` | `container`, `force?` | destructive | Remove a container (opt-in). |

## Kubernetes cluster

Cluster control via the API (kube-rs, reads kubeconfig; no `kubectl`), gated by
`[kubernetes]` — on by default; `k8s_delete` hidden unless `allow_destructive`.
Full detail: [containers.md](containers.md#kubernetes-cluster).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `k8s_contexts` | — | read | List kubeconfig contexts + current. |
| `k8s_get` | `kind`, `name?`, `namespace?` | read | Get one object or list a kind. |
| `k8s_describe` | `kind`, `name`, `namespace?` | read | Full JSON of one object. |
| `k8s_logs` | `pod`, `namespace?`, `container?`, `tail?` | read | A pod's logs. |
| `k8s_apply` | `manifest` | write | Apply a kubefile (multi-doc YAML). |
| `k8s_scale` | `kind`, `name`, `replicas`, `namespace?` | write | Scale a workload. |
| `k8s_delete` | `kind`, `name`, `namespace?` | destructive | Delete a resource (opt-in). |

## Date & time

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `datetime` | `timezone?` | Current date/time (local + UTC + Unix), plus an optional IANA timezone. The model has no "now". |
| `date_diff` | `from`, `to?` | Difference between two dates (days/years, 'ago / from now'); `to` defaults to now. |
| `time_convert` | `time`, `to_tz`, `from_tz?` | Convert a date/time to another IANA timezone. |

## Language

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `translate` | `text`, `to`, `from?` | Translate text via Google Translate (keyless); `to` is an ISO-639 code, `from` auto-detects. |
| `detect_language` | `text` | Detect a text's language (ISO-639 code) via Google Translate (keyless). |

## Data, math & units (local, no network)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `json_query` | `json`, `pointer?` | Parse/validate JSON; extract a value by RFC-6901 JSON Pointer, or pretty-print. |
| `json_format` | `json`, `minify?` | Pretty-print (default) or minify JSON. |
| `yaml_to_json` | `data` | Convert YAML → JSON. |
| `json_to_yaml` | `data` | Convert JSON → YAML. |
| `regex_search` | `pattern`, `text`, `all?`, `ignore_case?` | Find regex matches and their capture groups (Rust `regex` syntax). |
| `regex_replace` | `pattern`, `text`, `replacement`, `all?`, `ignore_case?` | Substitute regex matches (`$1`/`${name}` refs). |
| `math_eval` | `expression` | Evaluate an arithmetic/scientific expression (sqrt, sin, pi, `^`, …). |
| `math_solve` | `equation` | Solve a linear/quadratic equation in `x` (e.g. `x^2 - 5x + 6 = 0`). |
| `convert_units` | `value`, `from`, `to` | Convert between units (length/mass/volume/area/speed/time/data/temperature). |

## Meta

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `list_providers` | — | Show the active providers, strategy, and ranking. |
| `hive_status` | — | Show the peer-to-peer hivemind graph (peers, reputation, edges); says disabled when off. |

## Per-provider

One direct tool per *configured* provider, named `<kind>_<id>` (e.g. `web_mojeek`,
`code_github`, `qa_stackoverflow`, `docs_react`, `docs_kubernetes`). Args: `query`,
`max_results?`, `language?`, `site?`, `render?`. Targets a single source, bypassing
the chain/strategy. Generated from your config and gateable via `[tools]`.

StackOverflow adds one bespoke provider skill: `qa_stackoverflow_answers`
(`question`, `site?`, `max_answers?`) — read a question's body + top answers (with
code).

---

**Typical flow:** *search* (`web_search` / `code_search` / `docs_search` /
`qa_search`) → *retrieve* the best hit (`fetch_page` / `render_page` /
`fetch_repo_file` / `qa_stackoverflow_answers`).
