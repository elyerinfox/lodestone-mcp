//! Memory, solutions, and learned synonyms — persisted across sessions in a
//! local SQLite database (`{dir}/store.db`, default `.lodestone-memory/`).
//!
//! Three related tool families share one database:
//!
//! * **`memory_*`** — a key→value store the model can write to remember
//!   anything across sessions (`save`/`get`/`list`/`search`/`forget`). Optional
//!   `scope` namespaces and `tags`.
//! * **`solution_*`** — proposed solutions to past problems, with full revision
//!   history, **typed relation graph** (`supersedes`/`depends-on`/`related-to`
//!   etc. — `link`/`unlink`/`graph`/`related`), and tag-aware fuzzy recall
//!   (`record`/`find`/`show`/`list`/`update`/`forget`). `solution_find` is
//!   **advisory**, never prescriptive — it surfaces matching prior entries as
//!   suggestions ranked by exact canonical > exact concept > fuzzy Jaccard >
//!   substring, plus a tag boost.
//! * **`synonym_*`** — single-token aliases the model accumulates as it learns
//!   (`add`/`remove`/`list`). Loaded into a shared in-memory map at startup and
//!   read by `canonical_query` / `concept_tokens` in `src/provider.rs`, so
//!   *both* the search cache and the memory recall benefit. Empty out of the
//!   box — no hardcoded table.
//!
//! Storage is **proper indexed SQLite** (via `sqlx`), so the layer scales to
//! millions of rows with bounded per-page partitioning, ACID writes, and
//! transactional deletes (CASCADE removes a solution's revisions/tags/links;
//! dangling incoming edges are cleaned in the same transaction). Schema
//! changes are versioned via a **migration system**: each migration is a SQL
//! file in `migrations/`, compiled into the binary at build time and applied
//! transactionally at startup if its version is greater than the highest
//! already-applied one (tracked in a `_schema_version` table).
//!
//! On by default (`[memory].enabled = true`). Entries live **only on this
//! host** — never advertised in the constellation digest. `*_forget` are
//! routed through the confirmation [`guard`](super::guard).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};
use sqlx::FromRow;

use crate::skills::guard::Decision;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{config, internal, invalid, text_result};

/// Tool names (gated by `[memory].enabled` in `disabled_by_config`).
pub const TOOL_NAMES: &[&str] = &[
    // Memory
    "memory_save",
    "memory_get",
    "memory_list",
    "memory_search",
    "memory_forget",
    // Solutions
    "solution_record",
    "solution_find",
    "solution_show",
    "solution_list",
    "solution_update",
    "solution_forget",
    // Solution graph
    "solution_link",
    "solution_unlink",
    "solution_graph",
    "solution_related",
    // Solution phrasings (alt-phrasing recall + semantic search)
    "solution_alias_add",
    "solution_alias_remove",
    // Learned synonyms
    "synonym_add",
    "synonym_remove",
    "synonym_list",
    // Conversation traversal — read-only.
    "conversation_list",
    "conversation_show",
    "solution_conversations",
    // Conversation destructive controls.
    "conversation_forget",
    "conversation_prune",
];

const DEFAULT_DIR: &str = ".lodestone-memory";
const DB_FILE: &str = "store.db";

// ---------------------------------------------------------------------------
// Embeddings — optional OpenAI-compatible /v1/embeddings client + cosine.
// ---------------------------------------------------------------------------

/// Cosine similarity in [-1.0, 1.0]. Returns 0.0 when either vector is zero
/// or the lengths differ.
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Pack a vector as `[u32 LE dim][f32 LE]*dim` for BLOB storage. Length-
/// prefixed so a later config change to a different-dim model can be
/// detected at read time (rows with mismatched dim are ignored, not crashed).
fn embedding_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + v.len() * 4);
    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Decode a BLOB written by [`embedding_to_blob`]. Returns `None` on a length
/// mismatch or a malformed prefix.
fn blob_to_embedding(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() < 4 {
        return None;
    }
    let dim = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    if b.len() != 4 + dim * 4 {
        return None;
    }
    let mut out = Vec::with_capacity(dim);
    for i in 0..dim {
        let off = 4 + i * 4;
        out.push(f32::from_le_bytes([
            b[off],
            b[off + 1],
            b[off + 2],
            b[off + 3],
        ]));
    }
    Some(out)
}

/// Fetch an embedding for `text` from an OpenAI-compatible
/// `/v1/embeddings` endpoint. Returns `None` when embeddings are disabled
/// (`endpoint.is_empty()`) or the call fails — callers must treat this as
/// "no semantic recall for this row" rather than a hard error.
async fn fetch_embedding(
    http: &reqwest::Client,
    endpoint: &str,
    model: &str,
    text: &str,
) -> Option<Vec<f32>> {
    if endpoint.trim().is_empty() || text.trim().is_empty() {
        return None;
    }
    let body = serde_json::json!({ "input": text, "model": model });
    let resp = http
        .post(endpoint)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    let v: serde_json::Value = resp.json().await.ok()?;
    let arr = v.pointer("/data/0/embedding").and_then(|x| x.as_array())?;
    let mut out = Vec::with_capacity(arr.len());
    for x in arr {
        out.push(x.as_f64()? as f32);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// Schema migrations
// ---------------------------------------------------------------------------

/// One ordered, idempotent schema change. The SQL is embedded at build time so
/// the running binary is fully self-contained — there's no separate
/// `migrations/` directory to ship next to it. To add a migration: write
/// `migrations/000N_<name>.sql` and append a `Migration { version: N, … }`
/// entry to `MIGRATIONS`. The runner applies it inside a transaction and
/// records `N` in `_schema_version`.
struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("../../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "conversations",
        sql: include_str!("../../migrations/0002_conversations.sql"),
    },
    Migration {
        version: 3,
        name: "embeddings",
        sql: include_str!("../../migrations/0003_embeddings.sql"),
    },
];

/// Apply every migration whose version is greater than the highest already
/// recorded in `_schema_version`. Each migration runs in its own transaction
/// so a partial failure doesn't leave the database half-migrated.
async fn apply_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS _schema_version (\
            version    INTEGER NOT NULL PRIMARY KEY,\
            applied_at INTEGER NOT NULL,\
            name       TEXT    NOT NULL\
        )",
    )
    .execute(pool)
    .await
    .context("create _schema_version")?;
    let current: Option<(i64,)> =
        sqlx::query_as("SELECT COALESCE(MAX(version), 0) FROM _schema_version")
            .fetch_optional(pool)
            .await
            .context("read current schema version")?;
    let current_v = current.map(|c| c.0 as u32).unwrap_or(0);
    for m in MIGRATIONS {
        if m.version > current_v {
            tracing::info!("memory: applying migration v{} ({})", m.version, m.name);
            let mut tx = pool
                .begin()
                .await
                .with_context(|| format!("begin tx for migration v{}", m.version))?;
            sqlx::raw_sql(m.sql)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("apply migration v{} ({})", m.version, m.name))?;
            sqlx::query("INSERT INTO _schema_version (version, applied_at, name) VALUES (?, ?, ?)")
                .bind(m.version as i64)
                .bind(now_secs() as i64)
                .bind(m.name)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("record migration v{}", m.version))?;
            tx.commit()
                .await
                .with_context(|| format!("commit migration v{}", m.version))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fmt_ts(ts: u64) -> String {
    use chrono::{TimeZone, Utc};
    Utc.timestamp_opt(ts as i64, 0)
        .single()
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

fn truncate(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// Normalize a tag list: trim, drop empties, dedupe case-insensitively while
/// preserving the first-seen casing.
fn clean_tags(raw: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for t in raw {
        let trimmed = t.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let lc = trimmed.to_ascii_lowercase();
        if seen.insert(lc) {
            out.push(trimmed);
        }
    }
    out
}

fn concept_key_of(text: &str) -> Option<String> {
    let toks = crate::provider::concept_tokens(text);
    if toks.is_empty() {
        None
    } else {
        Some(toks.join(" "))
    }
}

/// Reciprocal relation kind. Known directional pairs flip; everything else is
/// treated as symmetric (same kind on both ends).
fn reciprocal_kind(kind: &str) -> String {
    match kind {
        "supersedes" => "superseded-by".to_string(),
        "superseded-by" => "supersedes".to_string(),
        "depends-on" => "dependency-of".to_string(),
        "dependency-of" => "depends-on".to_string(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// The shared store
// ---------------------------------------------------------------------------

/// Per-process tracker for the "active conversation." Set on the first turn
/// after an idle gap; subsequent calls within `IDLE_GAP` keep extending the
/// same conversation. Held in a `Mutex` because the dispatch wrapper writes
/// from every concurrent tool call.
#[derive(Debug, Clone)]
struct ActiveConversation {
    id: String,
    /// Wall-clock seconds since UNIX_EPOCH of the last recorded turn. Compared
    /// against `IDLE_GAP_SECS` to decide whether to rotate the id.
    last_seen_secs: u64,
}

/// Per-process monotonic counter that disambiguates same-second conversation
/// ids. Combined with `now_secs()`, the id is unique within a process for the
/// lifetime of the universe — and stays decipherable when traversing later.
static CONV_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The memory layer: a SQLite connection pool plus a fast in-memory mirror of
/// the synonyms table (so `canonical_query` doesn't hit the disk on every
/// search/recall token).
#[derive(Clone)]
pub(crate) struct Memory {
    cfg: Arc<config::Memory>,
    pool: SqlitePool,
    synonyms: Arc<RwLock<HashMap<String, String>>>,
    /// Active conversation tracker. `None` until the first tool call (or after
    /// a long idle gap rotates the id).
    active_conv: Arc<std::sync::Mutex<Option<ActiveConversation>>>,
}

/// One scored prior-solution hit returned by [`Memory::auto_recall`]. Used by
/// the dispatch wrapper to prepend "💡 Prior solutions" preambles to tool
/// results — the model doesn't have to call `solution_find` explicitly to
/// benefit from recorded prior work.
///
/// Carries the hit's outgoing typed links too, so the recall preamble exposes
/// the **local subgraph** (supersedes / depends-on / related-to chains) — not
/// just the single solution. The model can then decide to walk further with
/// `solution_graph` / `solution_related`, or chain solutions transitively when
/// deciding what to do next.
#[derive(Debug, Clone)]
pub(crate) struct RecallHit {
    pub id: String,
    pub problem: String,
    /// Final score used for ranking: `max(token_score, semantic_score)`.
    pub score: f64,
    /// Token-overlap path score (canonical / concept / fuzzy / substring +
    /// tag overlap). `0.0` if no token signal was present.
    pub token_score: f64,
    /// Semantic-cosine path score mapped onto a token-comparable range.
    /// `0.0` when the embedding endpoint is off, the query couldn't be
    /// embedded, or the cosine similarity was below
    /// `[memory].embedding_threshold`.
    pub semantic_score: f64,
    pub summary: String,
    /// Outgoing typed edges (kind, target id) up to a small cap. Surfacing
    /// these in the preamble is what makes the auto-recall *graph-aware*
    /// rather than per-solution-isolated.
    pub links: Vec<(String, String)>,
    /// If this hit is part of a `superseded-by` chain, the id at the head of
    /// the chain (the solution nothing has superseded). When `Some(head)` and
    /// `head != id`, the recall preamble loudly tells the model to use the
    /// head instead of this hit — the entire point of recording supersession
    /// is that the older one is *obsolete*, and we should never quietly
    /// surface obsolete prior work.
    pub superseded_by_head: Option<String>,
    /// Set by the dispatch wrapper when this hit was semantic-only and the
    /// query was auto-attached as a new phrasing on the solution. The
    /// preamble renders a small "(noted this phrasing for next time)"
    /// annotation so the model can see the system is learning from the
    /// interaction rather than the attach happening invisibly.
    pub auto_attached_as_phrasing: bool,
}

impl RecallHit {
    /// True when the hit's token-overlap score by itself wouldn't have
    /// cleared the recall threshold, but the embedding cosine *did* — i.e.
    /// the query was worded so differently from the solution's stored text
    /// that only semantic matching saved it. The dispatch wrapper uses this
    /// signal to decide whether to auto-attach the query as a phrasing.
    pub fn was_semantic_only(&self, recall_threshold: f64) -> bool {
        self.token_score < recall_threshold && self.semantic_score >= recall_threshold
    }
}

impl Memory {
    /// `true` if the `memory_*` / `solution_*` / `synonym_*` family is enabled —
    /// the dispatch wrapper checks this before running auto-recall so the cost
    /// is zero when memory is off.
    pub(crate) fn enabled(&self) -> bool {
        self.cfg.enabled
    }

    /// Read-only handle to the resolved `[memory]` config. Used by the
    /// dispatch wrapper to decide whether to run auto-recall, record
    /// conversation turns, etc.
    pub(crate) fn config(&self) -> &config::Memory {
        &self.cfg
    }

    /// Fetch an embedding for `text` if the embedding endpoint is configured
    /// and reachable. Best-effort: returns `None` on any failure, so callers
    /// degrade to token-only scoring without raising.
    pub(crate) async fn embed(&self, http: &reqwest::Client, text: &str) -> Option<Vec<f32>> {
        fetch_embedding(
            http,
            &self.cfg.embedding_endpoint,
            &self.cfg.embedding_model,
            text,
        )
        .await
    }

    /// Run the same scoring path `solution_find` uses, but return the top
    /// matches above a threshold as plain data (no rendering). Called from the
    /// dispatch wrapper on every query-bearing tool so prior solutions are
    /// surfaced **intrinsically** — the model never has to remember to look.
    ///
    /// Returns at most `max` hits with `score â‰¥ 30`. Threshold avoids drowning
    /// the response in marginal substring matches.
    pub(crate) async fn auto_recall(
        &self,
        http: &reqwest::Client,
        query: &str,
        max: usize,
    ) -> Vec<RecallHit> {
        if !self.cfg.enabled || query.trim().is_empty() || max == 0 {
            return Vec::new();
        }
        let qcanon = crate::provider::canonical_query(query);
        let qconcept_str = concept_key_of(query);
        let q_concept_toks = crate::provider::concept_tokens(query);
        let needle = query.trim().to_ascii_lowercase();
        let filter_tags_lc: HashSet<String> = HashSet::new();

        // Pull solutions with their (optional) embeddings. Phrasings are loaded
        // in a second pass so a solution's recall fires on ANY phrasing.
        let rows: Vec<SolutionWithEmbed> = match sqlx::query_as(
            "SELECT id, problem, canon_key, concept_key, created_at, updated_at, embedding \
             FROM solutions",
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let all_tags: Vec<(String, String)> =
            sqlx::query_as("SELECT solution_id, tag FROM solution_tags")
                .fetch_all(&self.pool)
                .await
                .unwrap_or_default();
        let mut tags_by_sol: HashMap<String, HashSet<String>> = HashMap::new();
        for (sid, tag) in all_tags {
            tags_by_sol.entry(sid).or_default().insert(tag);
        }
        // Phrasings, indexed by solution_id. Each contributes its own
        // canon_key/concept_key (for token scoring) and embedding (for
        // semantic scoring). A solution's hit is the *best* score across
        // its own problem text and every attached phrasing.
        let phrasing_rows: Vec<PhrasingRow> = sqlx::query_as(
            "SELECT solution_id, phrasing, canon_key, concept_key, embedding \
             FROM solution_phrasings",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();
        let mut phrasings_by_sol: HashMap<String, Vec<PhrasingRow>> = HashMap::new();
        for p in phrasing_rows {
            phrasings_by_sol
                .entry(p.solution_id.clone())
                .or_default()
                .push(p);
        }

        // Compute the query embedding once. None when embeddings are disabled
        // or the endpoint is unreachable — the semantic path simply skips.
        let q_emb = self.embed(http, query).await;

        let empty_set: HashSet<String> = HashSet::new();
        let mut scored: Vec<(SolutionRow, f64, f64, f64)> = rows
            .into_iter()
            .filter_map(|row| {
                let tags = tags_by_sol.get(&row.id).unwrap_or(&empty_set);
                let sol = SolutionRow {
                    id: row.id.clone(),
                    problem: row.problem.clone(),
                    canon_key: row.canon_key.clone(),
                    concept_key: row.concept_key.clone(),
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                };
                // Token score against the solution's own problem first.
                let mut token_score = score_solution_row(
                    &sol,
                    tags,
                    &qcanon,
                    qconcept_str.as_deref(),
                    &q_concept_toks,
                    &needle,
                    &filter_tags_lc,
                )
                .map(|(s, _)| s)
                .unwrap_or(0.0_f64);
                // Then against each attached phrasing — take the max so a
                // single rephrasing covers every prior wording.
                if let Some(phrs) = phrasings_by_sol.get(&row.id) {
                    for p in phrs {
                        let p_sol = SolutionRow {
                            id: row.id.clone(),
                            problem: p.phrasing.clone(),
                            canon_key: p.canon_key.clone(),
                            concept_key: p.concept_key.clone(),
                            created_at: row.created_at,
                            updated_at: row.updated_at,
                        };
                        if let Some((s, _)) = score_solution_row(
                            &p_sol,
                            tags,
                            &qcanon,
                            qconcept_str.as_deref(),
                            &q_concept_toks,
                            &needle,
                            &filter_tags_lc,
                        ) {
                            if s > token_score {
                                token_score = s;
                            }
                        }
                    }
                }
                // Semantic path: cosine similarity against the solution's
                // own embedding AND each phrasing's embedding, take the max.
                let mut semantic_score = 0.0_f64;
                if let Some(qv) = q_emb.as_ref() {
                    let mut sim_best = 0.0_f32;
                    if let Some(blob) = row.embedding.as_ref() {
                        if let Some(sv) = blob_to_embedding(blob) {
                            sim_best = cosine(qv, &sv).max(sim_best);
                        }
                    }
                    if let Some(phrs) = phrasings_by_sol.get(&row.id) {
                        for p in phrs {
                            if let Some(blob) = p.embedding.as_ref() {
                                if let Some(sv) = blob_to_embedding(blob) {
                                    sim_best = cosine(qv, &sv).max(sim_best);
                                }
                            }
                        }
                    }
                    if sim_best >= self.cfg.embedding_threshold {
                        // Map cosine [threshold, 1.0] linearly into a token-
                        // comparable score in [40, 100]. So a hit at exactly
                        // the threshold scores 40 (above the default 30
                        // recall floor — fires by itself), and a near-
                        // perfect match scores 100. The slope only depends
                        // on the configured threshold so operators can
                        // tighten or loosen without touching this formula.
                        let span = (1.0_f32 - self.cfg.embedding_threshold).max(1e-3);
                        let s =
                            40.0 + 60.0 * ((sim_best - self.cfg.embedding_threshold) / span) as f64;
                        semantic_score = s.clamp(40.0, 100.0);
                    }
                }
                let best = token_score.max(semantic_score);
                if best < self.cfg.recall_threshold {
                    None
                } else {
                    Some((sol, best, token_score, semantic_score))
                }
            })
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.0.updated_at.cmp(&a.0.updated_at))
        });
        let mut hits = Vec::with_capacity(max);
        for (sol, score, token_score, semantic_score) in scored.into_iter().take(max) {
            let summary: Option<(String,)> = sqlx::query_as(
                "SELECT summary FROM solution_revisions \
                 WHERE solution_id = ? ORDER BY rev DESC LIMIT 1",
            )
            .bind(&sol.id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
            // Pull the hit's outgoing typed links so the preamble shows
            // chained / related prior work, not just the isolated hit.
            let links: Vec<(String, String)> = sqlx::query_as(
                "SELECT kind, to_id FROM solution_links \
                 WHERE from_id = ? ORDER BY kind, to_id LIMIT 8",
            )
            .bind(&sol.id)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();
            // Walk the `superseded-by` chain from this hit forward until
            // nothing supersedes the current node. The head is the *current*
            // accepted solution — surfacing an obsolete hit without pointing
            // at the head would silently mislead the model.
            let superseded_by_head = self.walk_supersession_head(&sol.id).await;
            hits.push(RecallHit {
                id: sol.id,
                problem: sol.problem,
                score,
                token_score,
                semantic_score,
                summary: summary.map(|s| s.0).unwrap_or_default(),
                links,
                superseded_by_head,
                auto_attached_as_phrasing: false,
            });
        }
        hits
    }

    /// Attach a query as a new phrasing on `sol_id` and best-effort embed it
    /// so future semantic *and* token recall can find it. Returns `true`
    /// when a fresh row was inserted, `false` when the same phrasing was
    /// already attached (FNV-1a hash dedup). Errors are swallowed — this is
    /// invoked from the dispatch wrapper and must never break a tool call.
    pub(crate) async fn auto_attach_phrasing(
        &self,
        http: &reqwest::Client,
        sol_id: &str,
        phrasing: &str,
    ) -> bool {
        let trimmed = phrasing.trim();
        if trimmed.is_empty() {
            return false;
        }
        let canon = crate::provider::canonical_query(trimmed);
        if canon.is_empty() {
            return false;
        }
        let concept = concept_key_of(trimmed);
        let hash = phrasing_hash(&canon);
        let now = now_secs() as i64;
        let res = sqlx::query(
            "INSERT OR IGNORE INTO solution_phrasings \
             (solution_id, hash, phrasing, canon_key, concept_key, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(sol_id)
        .bind(&hash)
        .bind(trimmed)
        .bind(&canon)
        .bind(&concept)
        .bind(now)
        .execute(&self.pool)
        .await;
        let inserted = matches!(&res, Ok(r) if r.rows_affected() > 0);
        if inserted && !self.cfg.embedding_endpoint.trim().is_empty() {
            if let Some(vec) = self.embed(http, trimmed).await {
                let blob = embedding_to_blob(&vec);
                let _ = sqlx::query(
                    "UPDATE solution_phrasings SET embedding = ? \
                     WHERE solution_id = ? AND hash = ?",
                )
                .bind(blob)
                .bind(sol_id)
                .bind(&hash)
                .execute(&self.pool)
                .await;
            }
        }
        inserted
    }

    /// How many concept tokens does this query carry after stop-wording /
    /// synonym folding? The dispatch wrapper uses this to gate
    /// `auto_alias_on_semantic_recall` against `auto_alias_min_query_tokens`
    /// so a single common noun ("campus") doesn't get attached as a phrasing
    /// on whichever solution it semantically lands on.
    pub(crate) fn query_concept_token_count(&self, query: &str) -> usize {
        crate::provider::concept_tokens(query).len()
    }

    /// Live counts across the memory store. Used by the `features` tool to
    /// show operators / models what's actually been recorded; gracefully
    /// returns zeros when memory is disabled or the DB is unreachable.
    pub(crate) async fn stats(&self) -> MemoryStats {
        if !self.cfg.enabled {
            return MemoryStats::default();
        }
        async fn count(pool: &SqlitePool, sql: &str) -> i64 {
            sqlx::query_as::<_, (i64,)>(sql)
                .fetch_one(pool)
                .await
                .map(|r| r.0)
                .unwrap_or(0)
        }
        async fn count_nonnull(pool: &SqlitePool, table: &str, col: &str) -> i64 {
            let q = format!("SELECT COUNT(*) FROM {table} WHERE {col} IS NOT NULL");
            sqlx::query_as::<_, (i64,)>(&q)
                .fetch_one(pool)
                .await
                .map(|r| r.0)
                .unwrap_or(0)
        }
        MemoryStats {
            memos: count(&self.pool, "SELECT COUNT(*) FROM memory").await,
            solutions: count(&self.pool, "SELECT COUNT(*) FROM solutions").await,
            solution_revisions: count(&self.pool, "SELECT COUNT(*) FROM solution_revisions").await,
            solution_links: count(&self.pool, "SELECT COUNT(*) FROM solution_links").await,
            solution_tags: count(&self.pool, "SELECT COUNT(*) FROM solution_tags").await,
            solution_phrasings: count(&self.pool, "SELECT COUNT(*) FROM solution_phrasings").await,
            synonyms: count(&self.pool, "SELECT COUNT(*) FROM synonyms").await,
            conversations: count(&self.pool, "SELECT COUNT(*) FROM conversations").await,
            conversation_turns: count(&self.pool, "SELECT COUNT(*) FROM conversation_turns").await,
            solutions_embedded: count_nonnull(&self.pool, "solutions", "embedding").await,
            phrasings_embedded: count_nonnull(&self.pool, "solution_phrasings", "embedding").await,
        }
    }
}

/// Aggregate counts emitted by [`Memory::stats`].
#[derive(Debug, Default, Clone)]
pub(crate) struct MemoryStats {
    pub memos: i64,
    pub solutions: i64,
    pub solution_revisions: i64,
    pub solution_links: i64,
    pub solution_tags: i64,
    pub solution_phrasings: i64,
    pub synonyms: i64,
    pub conversations: i64,
    pub conversation_turns: i64,
    pub solutions_embedded: i64,
    pub phrasings_embedded: i64,
}

impl Memory {
    /// Decide which conversation id to attribute the current tool call to.
    /// Idle-gap heuristic: if the previous call was within
    /// `[memory].conversation_idle_gap_secs`, reuse the same id; otherwise
    /// mint a fresh one and INSERT the row.
    ///
    /// Returns `None` when memory is disabled OR conversation recording is
    /// turned off — callers should treat that as "no conversation context"
    /// and skip the bookkeeping entirely.
    pub(crate) async fn current_conversation_id(&self) -> Option<String> {
        if !self.cfg.enabled || !self.cfg.record_conversations {
            return None;
        }
        let now = now_secs();
        let gap = self.cfg.conversation_idle_gap_secs;
        // Decide under the lock whether to reuse or rotate. We don't hit the
        // DB while holding the lock — only the in-memory tracker.
        let (id, is_new) = {
            let mut guard = self.active_conv.lock().ok()?;
            match guard.as_mut() {
                Some(active) if now.saturating_sub(active.last_seen_secs) <= gap => {
                    active.last_seen_secs = now;
                    (active.id.clone(), false)
                }
                _ => {
                    let n = CONV_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let id = format!("conv-{now}-{n:04x}");
                    *guard = Some(ActiveConversation {
                        id: id.clone(),
                        last_seen_secs: now,
                    });
                    (id, true)
                }
            }
        };
        if is_new {
            // Best-effort INSERT — if the DB is gone or read-only the recall
            // path mustn't break; just return the in-memory id.
            let _ = sqlx::query(
                "INSERT OR IGNORE INTO conversations \
                 (id, started_at, last_seen_at, turn_count) VALUES (?, ?, ?, 0)",
            )
            .bind(&id)
            .bind(now as i64)
            .bind(now as i64)
            .execute(&self.pool)
            .await;
        }
        Some(id)
    }

    /// Append one turn to the active conversation. Called by the dispatch
    /// wrapper after every tool call when memory is enabled. Best-effort: a
    /// DB error must not break the user-visible response.
    pub(crate) async fn record_turn(
        &self,
        conv_id: &str,
        tool_name: &str,
        query: Option<&str>,
        response_excerpt: &str,
    ) {
        if !self.cfg.enabled || !self.cfg.record_conversations {
            return;
        }
        // `record_only_query_calls=true` keeps the log focused on intent:
        // a tool call with no free-text query (fs_read, arithmetic_eval,
        // docker_ps) doesn't get written. Recall is unaffected.
        if self.cfg.record_only_query_calls && query.unwrap_or("").trim().is_empty() {
            return;
        }
        let now = now_secs() as i64;
        let excerpt: String = response_excerpt
            .chars()
            .take(self.cfg.conversation_turn_excerpt_max_chars)
            .collect();
        // Try to compute the next seq + bump turn_count + last_seen_at +
        // first_query (if NULL) atomically. Worst case the unique constraint
        // fires and we drop this turn rather than corrupt the sequence.
        let mut tx = match self.pool.begin().await {
            Ok(t) => t,
            Err(_) => return,
        };
        let next_seq: i64 = sqlx::query_as::<_, (i64,)>(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM conversation_turns WHERE conversation_id = ?",
        )
        .bind(conv_id)
        .fetch_one(&mut *tx)
        .await
        .map(|r| r.0)
        .unwrap_or(1);
        let _ = sqlx::query(
            "INSERT INTO conversation_turns \
             (conversation_id, seq, ts, tool_name, query, response_excerpt) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(conv_id)
        .bind(next_seq)
        .bind(now)
        .bind(tool_name)
        .bind(query)
        .bind(&excerpt)
        .execute(&mut *tx)
        .await;
        let _ = sqlx::query(
            "UPDATE conversations SET turn_count = turn_count + 1, last_seen_at = ?, \
             first_query = COALESCE(first_query, ?) WHERE id = ?",
        )
        .bind(now)
        .bind(query)
        .bind(conv_id)
        .execute(&mut *tx)
        .await;
        let _ = tx.commit().await;
    }

    /// Apply the retention rules in `[memory]` (older-than-N-days and
    /// keep-newest-N) and return the number of conversations that would be /
    /// were deleted. Best-effort: a DB error mid-sweep returns whatever count
    /// we managed before the failure.
    ///
    /// When `dry_run` is true, no rows are deleted — the count reflects what
    /// *would* be removed. `solution_revisions.conversation_id` is set to
    /// NULL for any revision whose conversation is deleted, so historical
    /// solution data stays intact (just loses its back-pointer).
    pub(crate) async fn prune_conversations(
        &self,
        retention_days: u32,
        keep_newest: usize,
        dry_run: bool,
    ) -> Result<u64, sqlx::Error> {
        // 1) Build the ids-to-delete set in one pass.
        let mut to_delete: Vec<String> = Vec::new();
        if retention_days > 0 {
            let cutoff = now_secs().saturating_sub(retention_days as u64 * 86_400) as i64;
            let old: Vec<(String,)> =
                sqlx::query_as("SELECT id FROM conversations WHERE last_seen_at < ?")
                    .bind(cutoff)
                    .fetch_all(&self.pool)
                    .await?;
            to_delete.extend(old.into_iter().map(|r| r.0));
        }
        if keep_newest > 0 {
            // SQLite OFFSET on an ORDER BY last_seen_at DESC gives us
            // everything past the newest N. dedup with the above set.
            let extras: Vec<(String,)> = sqlx::query_as(
                "SELECT id FROM conversations ORDER BY last_seen_at DESC LIMIT -1 OFFSET ?",
            )
            .bind(keep_newest as i64)
            .fetch_all(&self.pool)
            .await?;
            for (id,) in extras {
                if !to_delete.contains(&id) {
                    to_delete.push(id);
                }
            }
        }
        if dry_run || to_delete.is_empty() {
            return Ok(to_delete.len() as u64);
        }
        // 2) Apply in a transaction. CASCADE removes turns; we explicitly
        //    NULL revisions' back-pointer so solution_show / list still work.
        let mut tx = self.pool.begin().await?;
        for id in &to_delete {
            sqlx::query(
                "UPDATE solution_revisions SET conversation_id = NULL WHERE conversation_id = ?",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM conversations WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        // 3) If the active conversation was just deleted, rotate next call.
        if let Ok(mut guard) = self.active_conv.lock() {
            if let Some(active) = guard.as_ref() {
                if to_delete.iter().any(|x| x == &active.id) {
                    *guard = None;
                }
            }
        }
        Ok(to_delete.len() as u64)
    }

    /// Delete exactly one conversation by id. Returns whether a row existed.
    /// Like [`prune_conversations`], NULLs `solution_revisions.conversation_id`
    /// for any revision that referenced it.
    pub(crate) async fn forget_conversation(&self, id: &str) -> Result<bool, sqlx::Error> {
        let existed: Option<(String,)> =
            sqlx::query_as("SELECT id FROM conversations WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        if existed.is_none() {
            return Ok(false);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE solution_revisions SET conversation_id = NULL WHERE conversation_id = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        if let Ok(mut guard) = self.active_conv.lock() {
            if let Some(active) = guard.as_ref() {
                if active.id == id {
                    *guard = None;
                }
            }
        }
        Ok(true)
    }

    /// Follow `superseded-by` edges from `start` forward until none remain,
    /// returning the id of the final (head) solution. Returns `None` when
    /// nothing supersedes `start`. Bounded to
    /// `[memory].superseded_walk_max_hops` and uses a visited set so cyclic
    /// data can't loop the recall path. When the cap is 0, supersession
    /// walking is effectively disabled (no warning is ever emitted).
    async fn walk_supersession_head(&self, start: &str) -> Option<String> {
        let max_hops = self.cfg.superseded_walk_max_hops;
        if max_hops == 0 {
            return None;
        }
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(start.to_string());
        let mut current = start.to_string();
        let mut found_any = false;
        for _ in 0..max_hops {
            let next: Option<(String,)> = sqlx::query_as(
                "SELECT to_id FROM solution_links \
                 WHERE from_id = ? AND kind = 'superseded-by' \
                 ORDER BY to_id LIMIT 1",
            )
            .bind(&current)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
            match next {
                Some((to,)) if !visited.contains(&to) => {
                    visited.insert(to.clone());
                    current = to;
                    found_any = true;
                }
                _ => break,
            }
        }
        if found_any {
            Some(current)
        } else {
            None
        }
    }

    /// Open the store: ensure the directory and SQLite file exist, run any
    /// pending migrations, load the synonyms table into a shared map, and
    /// install that map for `crate::provider`'s canonicalization to use.
    pub(crate) async fn new(cfg: config::Memory) -> anyhow::Result<Self> {
        let dir_str = if cfg.dir.trim().is_empty() {
            DEFAULT_DIR.to_string()
        } else {
            cfg.dir.clone()
        };
        let dir = PathBuf::from(&dir_str);
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create memory dir {}", dir.display()))?;
        let db_path = dir.join(DB_FILE);
        let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
            .with_context(|| format!("invalid sqlite path {}", db_path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);
        let pool = SqlitePool::connect_with(opts)
            .await
            .with_context(|| format!("open sqlite {}", db_path.display()))?;
        apply_migrations(&pool).await?;

        let rows: Vec<(String, String)> = sqlx::query_as("SELECT token, canonical FROM synonyms")
            .fetch_all(&pool)
            .await
            .context("load synonyms")?;
        let map: HashMap<String, String> = rows.into_iter().collect();
        // The global synonym store is install-once: if another `Memory` (or a
        // test) installed first, our fresh Arc would be orphaned and writes
        // through `mem.synonyms` would never reach `canonical_query`. Instead,
        // hand a candidate to `install_synonym_store`, take back the Arc that's
        // actually live, and merge our loaded rows into it so this DB's
        // synonyms are visible regardless of install order.
        let synonyms = crate::provider::install_synonym_store(Arc::new(RwLock::new(map.clone())));
        if let Ok(mut live) = synonyms.write() {
            for (k, v) in map {
                live.entry(k).or_insert(v);
            }
        }

        Ok(Self {
            cfg: Arc::new(cfg),
            pool,
            synonyms,
            active_conv: Arc::new(std::sync::Mutex::new(None)),
        })
    }
}

// ---------------------------------------------------------------------------
// memory_* tools
// ---------------------------------------------------------------------------

#[derive(FromRow)]
struct MemoryRow {
    key: String,
    scope: String,
    value: String,
    tags_json: String,
    created_at: i64,
    updated_at: i64,
}

impl MemoryRow {
    fn tags(&self) -> Vec<String> {
        serde_json::from_str(&self.tags_json).unwrap_or_default()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MemorySaveArgs {
    /// Unique key (within `scope`) for this memory.
    key: String,
    /// The text to remember (free-form; markdown OK).
    value: String,
    /// Optional grouping namespace (default ""). Same key in different scopes are distinct entries.
    #[serde(default)]
    scope: Option<String>,
    /// Optional tags (free-form; used by memory_search).
    #[serde(default)]
    tags: Option<Vec<String>>,
}

pub struct MemorySave;
impl Skill for MemorySave {
    fn name(&self) -> &'static str {
        "memory_save"
    }
    fn description(&self) -> &'static str {
        "Save a key→value memory that survives restarts and is reachable from future sessions. \
        Optional `scope` namespaces a group (e.g. \"user-prefs\") and optional `tags` make it \
        searchable via memory_search. Upserts if the key+scope already exists."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MemorySaveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MemorySaveArgs>()?;
            let mem = &server.memory;
            let key = args.key.trim();
            if key.is_empty() {
                return Err(invalid("key must not be empty"));
            }
            let cap = mem.cfg.max_value_chars.max(1);
            if args.value.chars().count() > cap {
                return Err(invalid(format!(
                    "value too long: {} chars (max {cap})",
                    args.value.chars().count()
                )));
            }
            let scope = args.scope.unwrap_or_default();
            let tags = clean_tags(args.tags.unwrap_or_default());
            let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".to_string());
            let now = now_secs() as i64;
            let prior: Option<(i64,)> =
                sqlx::query_as("SELECT created_at FROM memory WHERE scope = ? AND key = ?")
                    .bind(&scope)
                    .bind(key)
                    .fetch_optional(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            if prior.is_none() {
                let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memory")
                    .fetch_one(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
                if (count.0 as usize) >= mem.cfg.max_entries.max(1) {
                    return Err(invalid(format!(
                        "memory full: max_entries = {} (forget some to make room)",
                        mem.cfg.max_entries
                    )));
                }
            }
            sqlx::query(
                "INSERT INTO memory (scope, key, value, tags_json, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(scope, key) DO UPDATE SET value = excluded.value, \
                     tags_json = excluded.tags_json, updated_at = excluded.updated_at",
            )
            .bind(&scope)
            .bind(key)
            .bind(&args.value)
            .bind(&tags_json)
            .bind(prior.map(|p| p.0).unwrap_or(now))
            .bind(now)
            .execute(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let action = if prior.is_some() { "Updated" } else { "Saved" };
            Ok(text_result(format!(
                "{action} key=\"{key}\"{} at {}.",
                if scope.is_empty() {
                    String::new()
                } else {
                    format!(" scope=\"{scope}\"")
                },
                fmt_ts(now as u64)
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MemoryGetArgs {
    /// The key to fetch.
    key: String,
    /// Optional scope (default "").
    #[serde(default)]
    scope: Option<String>,
}

pub struct MemoryGet;
impl Skill for MemoryGet {
    fn name(&self) -> &'static str {
        "memory_get"
    }
    fn description(&self) -> &'static str {
        "Look up one saved memory by exact key (and optional scope). Returns the value with its \
        tags and timestamps, or a 'no such key' message."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MemoryGetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MemoryGetArgs>()?;
            let scope = args.scope.unwrap_or_default();
            let row: Option<MemoryRow> = sqlx::query_as(
                "SELECT key, scope, value, tags_json, created_at, updated_at \
                 FROM memory WHERE scope = ? AND key = ?",
            )
            .bind(&scope)
            .bind(&args.key)
            .fetch_optional(&server.memory.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let out = match row {
                None => format!(
                    "No memory for key=\"{}\"{}.",
                    args.key,
                    if scope.is_empty() {
                        String::new()
                    } else {
                        format!(" scope=\"{scope}\"")
                    }
                ),
                Some(e) => {
                    let tags = e.tags();
                    let mut s = format!(
                        "{}{}\n  saved {} · updated {}",
                        e.key,
                        if e.scope.is_empty() {
                            String::new()
                        } else {
                            format!(" [scope: {}]", e.scope)
                        },
                        fmt_ts(e.created_at as u64),
                        fmt_ts(e.updated_at as u64)
                    );
                    if !tags.is_empty() {
                        s.push_str(&format!("\n  tags: {}", tags.join(", ")));
                    }
                    s.push_str("\n\n");
                    s.push_str(&e.value);
                    s
                }
            };
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MemoryListArgs {
    /// Optional scope filter (default: all scopes).
    #[serde(default)]
    scope: Option<String>,
    /// Only show keys starting with this prefix.
    #[serde(default)]
    prefix: Option<String>,
    /// Max entries to show (default 25, capped at 200).
    #[serde(default)]
    max: Option<u32>,
}

pub struct MemoryList;
impl Skill for MemoryList {
    fn name(&self) -> &'static str {
        "memory_list"
    }
    fn description(&self) -> &'static str {
        "List saved memories — key, scope, tags, age, and a value preview. Optional `scope` and \
        `prefix` filters."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MemoryListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MemoryListArgs>()?;
            let max = args.max.unwrap_or(25).clamp(1, 200) as i64;
            let prefix_pat = args.prefix.as_ref().map(|p| format!("{p}%"));
            let q = match (&args.scope, &prefix_pat) {
                (Some(_), Some(_)) => {
                    "SELECT key, scope, value, tags_json, created_at, updated_at \
                                        FROM memory WHERE scope = ? AND key LIKE ? \
                                        ORDER BY updated_at DESC LIMIT ?"
                }
                (Some(_), None) => {
                    "SELECT key, scope, value, tags_json, created_at, updated_at \
                                    FROM memory WHERE scope = ? ORDER BY updated_at DESC LIMIT ?"
                }
                (None, Some(_)) => {
                    "SELECT key, scope, value, tags_json, created_at, updated_at \
                                    FROM memory WHERE key LIKE ? ORDER BY updated_at DESC LIMIT ?"
                }
                (None, None) => {
                    "SELECT key, scope, value, tags_json, created_at, updated_at \
                                 FROM memory ORDER BY updated_at DESC LIMIT ?"
                }
            };
            let mut query = sqlx::query_as::<_, MemoryRow>(q);
            if let Some(s) = &args.scope {
                query = query.bind(s);
            }
            if let Some(p) = &prefix_pat {
                query = query.bind(p);
            }
            query = query.bind(max);
            let rows = query
                .fetch_all(&server.memory.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            if rows.is_empty() {
                return Ok(text_result("No memories match.".to_string()));
            }
            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memory")
                .fetch_one(&server.memory.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            let mut out = format!(
                "{} memor{} ({} shown):\n",
                total.0,
                if total.0 == 1 { "y" } else { "ies" },
                rows.len()
            );
            for e in &rows {
                let tags = e.tags();
                let scope_tag = if e.scope.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", e.scope)
                };
                let tag_tag = if tags.is_empty() {
                    String::new()
                } else {
                    format!(" #{}", tags.join(" #"))
                };
                out.push_str(&format!(
                    "\n  {}{}{}\n    updated {} · {}\n",
                    e.key,
                    scope_tag,
                    tag_tag,
                    fmt_ts(e.updated_at as u64),
                    truncate(&e.value.replace('\n', " "), 100)
                ));
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MemorySearchArgs {
    /// Substring to look for in key, tags, or value (case-insensitive).
    query: String,
    /// Optional scope filter.
    #[serde(default)]
    scope: Option<String>,
    /// Optional single tag the entry must carry.
    #[serde(default)]
    tag: Option<String>,
    /// Max entries to show (default 10, capped at 50).
    #[serde(default)]
    max: Option<u32>,
}

pub struct MemorySearch;
impl Skill for MemorySearch {
    fn name(&self) -> &'static str {
        "memory_search"
    }
    fn description(&self) -> &'static str {
        "Search saved memories by substring (case-insensitive across key, tags, and value), with \
        optional `scope` and `tag` filters."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MemorySearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MemorySearchArgs>()?;
            let max = args.max.unwrap_or(10).clamp(1, 50) as i64;
            let needle_raw = args.query.trim();
            if needle_raw.is_empty() {
                return Err(invalid("empty query"));
            }
            let like_pat = format!("%{}%", needle_raw.to_ascii_lowercase());
            let q = "SELECT key, scope, value, tags_json, created_at, updated_at \
                     FROM memory \
                     WHERE (LOWER(key) LIKE ? OR LOWER(value) LIKE ? OR LOWER(tags_json) LIKE ?) \
                     ORDER BY updated_at DESC LIMIT ?";
            let rows: Vec<MemoryRow> = sqlx::query_as(q)
                .bind(&like_pat)
                .bind(&like_pat)
                .bind(&like_pat)
                .bind(max * 4) // overshoot, filter in Rust below
                .fetch_all(&server.memory.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            let scope_filter = args.scope.as_deref();
            let tag_filter_lc = args.tag.as_ref().map(|t| t.to_ascii_lowercase());
            let filtered: Vec<&MemoryRow> = rows
                .iter()
                .filter(|e| scope_filter.is_none_or(|s| e.scope == s))
                .filter(|e| match &tag_filter_lc {
                    Some(t) => e.tags().iter().any(|x| x.to_ascii_lowercase() == *t),
                    None => true,
                })
                .take(max as usize)
                .collect();
            if filtered.is_empty() {
                return Ok(text_result(format!(
                    "No memories match \"{}\".",
                    args.query
                )));
            }
            let mut out = format!(
                "{} memor{} match:\n",
                filtered.len(),
                if filtered.len() == 1 { "y" } else { "ies" }
            );
            for e in filtered {
                let scope_tag = if e.scope.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", e.scope)
                };
                out.push_str(&format!(
                    "\n  {}{}\n    updated {}\n    {}\n",
                    e.key,
                    scope_tag,
                    fmt_ts(e.updated_at as u64),
                    truncate(&e.value.replace('\n', " "), 140)
                ));
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MemoryForgetArgs {
    /// The key to forget.
    key: String,
    /// Optional scope (default "").
    #[serde(default)]
    scope: Option<String>,
    /// Confirmation token returned by the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// Whitelist `memory_forget` for the rest of the session (use with `confirm`).
    #[serde(default)]
    trust: Option<bool>,
}

pub struct MemoryForget;
impl Skill for MemoryForget {
    fn name(&self) -> &'static str {
        "memory_forget"
    }
    fn description(&self) -> &'static str {
        "Delete one saved memory by key+scope. Destructive — the first call returns a confirm token; \
        call again with confirm=<token> (or confirm + trust=true to whitelist for the session). \
        `[memory].allow_destructive=true` pre-authorizes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MemoryForgetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MemoryForgetArgs>()?;
            let mem = &server.memory;
            let scope = args.scope.clone().unwrap_or_default();
            let exists: Option<(i64,)> =
                sqlx::query_as("SELECT created_at FROM memory WHERE scope = ? AND key = ?")
                    .bind(&scope)
                    .bind(&args.key)
                    .fetch_optional(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            if exists.is_none() {
                return Ok(text_result(format!(
                    "No memory for key=\"{}\"{} — nothing to forget.",
                    args.key,
                    if scope.is_empty() {
                        String::new()
                    } else {
                        format!(" scope=\"{scope}\"")
                    }
                )));
            }
            if let Decision::Challenge(msg) = server.guard.check(
                &format!("memory_forget|{}|{}", scope, args.key),
                "memory_forget",
                mem.cfg.allow_destructive,
                &format!(
                    "delete memory key=\"{}\"{}",
                    args.key,
                    if scope.is_empty() {
                        String::new()
                    } else {
                        format!(" scope=\"{scope}\"")
                    }
                ),
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            sqlx::query("DELETE FROM memory WHERE scope = ? AND key = ?")
                .bind(&scope)
                .bind(&args.key)
                .execute(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            Ok(text_result(format!(
                "Forgot key=\"{}\"{}.",
                args.key,
                if scope.is_empty() {
                    String::new()
                } else {
                    format!(" scope=\"{scope}\"")
                }
            )))
        })
    }
}

// ---------------------------------------------------------------------------
// solution_* tools
// ---------------------------------------------------------------------------

#[derive(FromRow, Clone)]
struct SolutionRow {
    id: String,
    problem: String,
    canon_key: String,
    concept_key: Option<String>,
    created_at: i64,
    updated_at: i64,
}

/// Row variant used by `auto_recall` — same as `SolutionRow` plus the
/// optional `embedding` BLOB so we can score semantically in one query.
#[derive(FromRow)]
struct SolutionWithEmbed {
    id: String,
    problem: String,
    canon_key: String,
    concept_key: Option<String>,
    created_at: i64,
    updated_at: i64,
    embedding: Option<Vec<u8>>,
}

/// Row variant used by `auto_recall` — one alternate phrasing attached to a
/// solution, with its own token keys and (optional) embedding.
#[derive(FromRow)]
struct PhrasingRow {
    solution_id: String,
    phrasing: String,
    canon_key: String,
    concept_key: Option<String>,
    embedding: Option<Vec<u8>>,
}

#[derive(FromRow)]
struct RevisionRow {
    rev: i64,
    ts: i64,
    summary: String,
    content: String,
    notes: String,
    conversation_id: Option<String>,
}

async fn load_solution_tags(pool: &SqlitePool, id: &str) -> Result<Vec<String>, McpError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT label FROM solution_tags WHERE solution_id = ? ORDER BY tag")
            .bind(id)
            .fetch_all(pool)
            .await
            .map_err(|e| internal(e.into()))?;
    Ok(rows.into_iter().map(|t| t.0).collect())
}

#[derive(FromRow)]
struct LinkRow {
    kind: String,
    to_id: String,
    note: String,
}

async fn load_solution_links(pool: &SqlitePool, id: &str) -> Result<Vec<LinkRow>, McpError> {
    sqlx::query_as(
        "SELECT kind, to_id, note FROM solution_links \
         WHERE from_id = ? ORDER BY kind, to_id",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e| internal(e.into()))
}

async fn next_solution_id(pool: &mut sqlx::SqliteConnection) -> Result<String, McpError> {
    let n: (i64,) = sqlx::query_as(
        "SELECT COALESCE(MAX(CAST(SUBSTR(id, 5) AS INTEGER)), 0) + 1 \
         FROM solutions WHERE id GLOB 'sol-*'",
    )
    .fetch_one(&mut *pool)
    .await
    .map_err(|e| internal(e.into()))?;
    Ok(format!("sol-{}", n.0))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionRecordArgs {
    /// The problem / question this solution addresses (free text).
    problem: String,
    /// A one-line summary of the approach.
    summary: String,
    /// The full proposed solution (steps, code, links — markdown OK).
    content: String,
    /// Optional caveats, follow-ups, or what didn't work.
    #[serde(default)]
    notes: Option<String>,
    /// Optional free-form tags (e.g. ["deployment", "nginx", "tls"]).
    #[serde(default)]
    tags: Option<Vec<String>>,
}

pub struct SolutionRecord;
impl Skill for SolutionRecord {
    fn name(&self) -> &'static str {
        "solution_record"
    }
    fn description(&self) -> &'static str {
        "Record a proposed SOLUTION to a problem, persisted across sessions. Returns a solution id. \
        Later, solution_find will surface this entry as a SUGGESTION on similar questions (never \
        prescriptive). Use solution_update to append a revision when the approach changes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionRecordArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionRecordArgs>()?;
            let mem = &server.memory;
            let cap = mem.cfg.max_value_chars.max(1);
            if args.content.chars().count() > cap {
                return Err(invalid(format!(
                    "content too long: {} chars (max {cap})",
                    args.content.chars().count()
                )));
            }
            let now = now_secs() as i64;
            let canon = crate::provider::canonical_query(&args.problem);
            let concept = concept_key_of(&args.problem);
            let tags = clean_tags(args.tags.unwrap_or_default());
            let mut tx = mem.pool.begin().await.map_err(|e| internal(e.into()))?;
            let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM solutions")
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| internal(e.into()))?;
            if (count.0 as usize) >= mem.cfg.max_entries.max(1) {
                return Err(invalid(format!(
                    "solution store full: max_entries = {}",
                    mem.cfg.max_entries
                )));
            }
            let id = next_solution_id(&mut tx).await?;
            // Stamp the active conversation on this revision so we can later
            // answer "what conversation was this solution a part of?"
            let conv_id = mem.current_conversation_id().await;
            sqlx::query(
                "INSERT INTO solutions (id, problem, canon_key, concept_key, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&args.problem)
            .bind(&canon)
            .bind(&concept)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            sqlx::query(
                "INSERT INTO solution_revisions (solution_id, rev, ts, summary, content, notes, conversation_id) \
                 VALUES (?, 1, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(now)
            .bind(&args.summary)
            .bind(&args.content)
            .bind(args.notes.unwrap_or_default())
            .bind(conv_id.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            for tag in &tags {
                sqlx::query("INSERT INTO solution_tags (solution_id, tag, label) VALUES (?, ?, ?)")
                    .bind(&id)
                    .bind(tag.to_ascii_lowercase())
                    .bind(tag)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| internal(e.into()))?;
            }
            tx.commit().await.map_err(|e| internal(e.into()))?;
            // Best-effort embedding fetch — outside the tx so a slow / down
            // embedding server can't hold the write open. embedding=NULL is a
            // legitimate state (semantic recall just skips this solution).
            if !mem.cfg.embedding_endpoint.trim().is_empty() {
                let text = format!("{}\n\n{}", args.problem, args.summary);
                if let Some(vec) = mem.embed(&server.http, &text).await {
                    let blob = embedding_to_blob(&vec);
                    let _ = sqlx::query("UPDATE solutions SET embedding = ? WHERE id = ?")
                        .bind(blob)
                        .bind(&id)
                        .execute(&mem.pool)
                        .await;
                }
            }
            let tag_tail = if tags.is_empty() {
                String::new()
            } else {
                format!(" Tags: {}.", tags.join(", "))
            };
            Ok(text_result(format!(
                "Recorded {id} at {} (rev 1).{tag_tail} solution_find will surface it on similar questions.",
                fmt_ts(now as u64)
            )))
        })
    }
}

/// Score one stored solution row against a parsed query. Higher is better.
fn score_solution_row(
    sol: &SolutionRow,
    sol_tags_lc: &HashSet<String>,
    qcanon: &str,
    qconcept_str: Option<&str>,
    q_concept_toks: &[String],
    needle: &str,
    filter_tags_lc: &HashSet<String>,
) -> Option<(f64, &'static str)> {
    let mut best: Option<(f64, &'static str)> = None;
    let mut consider = |score: f64, label: &'static str| {
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, label));
        }
    };
    if !qcanon.is_empty() && sol.canon_key == qcanon {
        consider(100.0, "exact canonical");
    }
    if let (Some(qc), Some(sc)) = (qconcept_str, sol.concept_key.as_deref()) {
        if qc == sc {
            consider(80.0, "concept exact");
        }
    }
    if !q_concept_toks.is_empty() {
        if let Some(sc) = &sol.concept_key {
            let s_toks: HashSet<&str> = sc.split_whitespace().collect();
            let q_toks: HashSet<&str> = q_concept_toks.iter().map(|s| s.as_str()).collect();
            let inter = s_toks.intersection(&q_toks).count();
            if inter > 0 {
                // Two complementary measures of fuzzy match. We take the
                // stronger of the two so neither shape of query suffers:
                //   * Jaccard (intersection / union) is good for rich queries
                //     of comparable size to the solution's concept key.
                //   * Query-coverage (intersection / |query|) handles short
                //     focused queries — "Seattle, WA" on a 10-token solution
                //     would score Jaccard 1/10=0.1, but query-coverage 1/1=1.0
                //     correctly reflects "this query is entirely covered."
                // Without query-coverage, single-noun queries like the kind a
                // tool-using model emits when geocoding never clear the recall
                // threshold even when the match is obvious.
                let union = s_toks.union(&q_toks).count().max(1);
                let jaccard = inter as f64 / union as f64;
                let coverage = inter as f64 / q_toks.len().max(1) as f64;
                let strength = jaccard.max(coverage);
                if strength < 1.0 {
                    consider(20.0 + 40.0 * strength, "fuzzy overlap");
                } else {
                    // Both q_toks ⊆ s_toks and q_toks == s_toks land here;
                    // the latter is already caught by the "concept exact"
                    // path higher up. For the strict-subset case (every
                    // query token present but solution has more) score just
                    // under the exact-concept path so we don't dilute it.
                    consider(60.0, "fuzzy overlap");
                }
            }
        }
    }
    if !needle.is_empty() && sol.problem.to_ascii_lowercase().contains(needle) {
        consider(15.0, "substring");
    }
    // Tag overlap path:
    //   * If the caller passed `tags=[…]`, count tags that intersect that
    //     explicit filter (`solution_find tags=…` users get this).
    //   * Otherwise (auto-recall, plain `solution_find query=…`), count tags
    //     that intersect the query's significant concept tokens — so tags
    //     pull their weight as match signal even without an explicit filter.
    //     Tags are how the model labels what a solution is *about*; an
    //     overlap with the query is a meaningful score signal we were
    //     previously ignoring.
    let tag_overlap = if !filter_tags_lc.is_empty() {
        sol_tags_lc
            .iter()
            .filter(|t| filter_tags_lc.contains(*t))
            .count()
    } else if !q_concept_toks.is_empty() {
        let q_toks: HashSet<&str> = q_concept_toks.iter().map(|s| s.as_str()).collect();
        sol_tags_lc
            .iter()
            .filter(|t| q_toks.contains(t.as_str()))
            .count()
    } else {
        0
    };
    if let Some((score, label)) = best.as_mut() {
        if tag_overlap > 0 {
            *score += 5.0 * tag_overlap as f64;
        }
        Some((*score, *label))
    } else if tag_overlap > 0 {
        Some((10.0 + 5.0 * tag_overlap as f64, "tag"))
    } else {
        None
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionFindArgs {
    /// A question or problem description; matched against recorded problems. Optional
    /// if `tags` is given (then it's a tag-only browse).
    #[serde(default)]
    query: Option<String>,
    /// Optional tag filter — solutions carrying *any* of these tags surface as well
    /// (case-insensitive). Pure tag lookups (no query) are supported.
    #[serde(default)]
    tags: Option<Vec<String>>,
    /// Max suggestions to surface (default 5, capped at 20).
    #[serde(default)]
    max: Option<u32>,
}

pub struct SolutionFind;
impl Skill for SolutionFind {
    fn name(&self) -> &'static str {
        "solution_find"
    }
    fn description(&self) -> &'static str {
        "Find prior recorded solutions whose problem is similar to `query`. Returns each as a \
        SUGGESTION (advisory, NOT prescriptive — verify it still applies, then revise via \
        solution_update). Ranks by: exact canonical match > exact concept match > FUZZY concept \
        overlap (Jaccard over stemmed token sets) > substring; plus a boost for shared `tags`. \
        Either `query` or `tags` is required."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionFindArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionFindArgs>()?;
            let mem = &server.memory;
            let max = args.max.unwrap_or(5).clamp(1, 20) as usize;
            let query = args.query.unwrap_or_default();
            let filter_tags = args.tags.unwrap_or_default();
            if query.trim().is_empty() && filter_tags.is_empty() {
                return Err(invalid("supply at least one of `query` or `tags`"));
            }
            let qcanon = if query.trim().is_empty() {
                String::new()
            } else {
                crate::provider::canonical_query(&query)
            };
            let qconcept_str = concept_key_of(&query);
            let q_concept_toks: Vec<String> = if query.trim().is_empty() {
                Vec::new()
            } else {
                crate::provider::concept_tokens(&query)
            };
            let needle = query.trim().to_ascii_lowercase();
            let filter_tags_lc: HashSet<String> = filter_tags
                .iter()
                .map(|t| t.trim().to_ascii_lowercase())
                .filter(|t| !t.is_empty())
                .collect();
            let rows: Vec<SolutionRow> = sqlx::query_as(
                "SELECT id, problem, canon_key, concept_key, created_at, updated_at FROM solutions",
            )
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let all_tags: Vec<(String, String)> =
                sqlx::query_as("SELECT solution_id, tag FROM solution_tags")
                    .fetch_all(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            let mut tags_by_sol: HashMap<String, HashSet<String>> = HashMap::new();
            for (sid, tag) in all_tags {
                tags_by_sol.entry(sid).or_default().insert(tag);
            }
            let empty_set: HashSet<String> = HashSet::new();
            let mut ranked: Vec<(SolutionRow, f64, &'static str)> = rows
                .into_iter()
                .filter_map(|sol| {
                    let sol_tags = tags_by_sol.get(&sol.id).unwrap_or(&empty_set);
                    score_solution_row(
                        &sol,
                        sol_tags,
                        &qcanon,
                        qconcept_str.as_deref(),
                        &q_concept_toks,
                        &needle,
                        &filter_tags_lc,
                    )
                    .map(|(score, label)| (sol, score, label))
                })
                .collect();
            if ranked.is_empty() {
                return Ok(text_result(format!(
                    "No prior solutions match{}{}. (Record one with solution_record once you've worked it out.)",
                    if query.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" \"{}\"", query.trim())
                    },
                    if filter_tags_lc.is_empty() {
                        String::new()
                    } else {
                        format!(" with tags [{}]", filter_tags.join(", "))
                    },
                )));
            }
            ranked.sort_by(|a, b| {
                b.1.partial_cmp(&a.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.0.updated_at.cmp(&a.0.updated_at))
            });
            let total = ranked.len();
            let mut out = format!(
                "{} SUGGESTED prior solution{} (advisory — may be stale; verify before reusing):\n",
                total,
                if total == 1 { "" } else { "s" }
            );
            for (i, (sol, score, label)) in ranked.iter().take(max).enumerate() {
                let last: Option<(i64, String)> = sqlx::query_as(
                    "SELECT rev, summary FROM solution_revisions \
                     WHERE solution_id = ? ORDER BY rev DESC LIMIT 1",
                )
                .bind(&sol.id)
                .fetch_optional(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
                let sol_tags = load_solution_tags(&mem.pool, &sol.id).await?;
                let tags_line = if sol_tags.is_empty() {
                    String::new()
                } else {
                    format!("   Tags: {}\n", sol_tags.join(", "))
                };
                out.push_str(&format!(
                    "\n{}. {}  (rev {}, updated {})\n   Problem: {}\n   Match: {} (score {:.1})\n{}",
                    i + 1,
                    sol.id,
                    last.as_ref().map(|r| r.0).unwrap_or(0),
                    fmt_ts(sol.updated_at as u64),
                    truncate(&sol.problem.replace('\n', " "), 140),
                    label,
                    score,
                    tags_line,
                ));
                if let Some(r) = &last {
                    out.push_str(&format!("   Latest summary: {}\n", truncate(&r.1, 200)));
                }
                out.push_str(&format!(
                    "   (solution_show id=\"{}\" for the full history)\n",
                    sol.id
                ));
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionShowArgs {
    /// Solution id (e.g. "sol-3").
    id: String,
}

pub struct SolutionShow;
impl Skill for SolutionShow {
    fn name(&self) -> &'static str {
        "solution_show"
    }
    fn description(&self) -> &'static str {
        "Show one recorded solution by id, with its full revision history (oldest to newest), \
        its tags, and its outbound links."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionShowArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionShowArgs>()?;
            let mem = &server.memory;
            let sol: Option<SolutionRow> = sqlx::query_as(
                "SELECT id, problem, canon_key, concept_key, created_at, updated_at \
                 FROM solutions WHERE id = ?",
            )
            .bind(&args.id)
            .fetch_optional(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let Some(sol) = sol else {
                return Err(invalid(format!("no solution \"{}\"", args.id)));
            };
            let revs: Vec<RevisionRow> = sqlx::query_as(
                "SELECT rev, ts, summary, content, notes, conversation_id FROM solution_revisions \
                 WHERE solution_id = ? ORDER BY rev ASC",
            )
            .bind(&sol.id)
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let tags = load_solution_tags(&mem.pool, &sol.id).await?;
            let links = load_solution_links(&mem.pool, &sol.id).await?;
            let mut out = format!(
                "{} — {} revision{}\nProblem: {}\nFirst recorded: {}\nLast updated: {}\n",
                sol.id,
                revs.len(),
                if revs.len() == 1 { "" } else { "s" },
                sol.problem,
                fmt_ts(sol.created_at as u64),
                fmt_ts(sol.updated_at as u64),
            );
            if !tags.is_empty() {
                out.push_str(&format!("Tags: {}\n", tags.join(", ")));
            }
            if !links.is_empty() {
                out.push_str("Links:\n");
                for l in &links {
                    let note = if l.note.is_empty() {
                        String::new()
                    } else {
                        format!("  — {}", l.note)
                    };
                    out.push_str(&format!("  ─{}→ {}{}\n", l.kind, l.to_id, note));
                }
            }
            for r in &revs {
                let conv_tail = match r.conversation_id.as_deref() {
                    Some(c) => format!(" · {c}"),
                    None => String::new(),
                };
                out.push_str(&format!(
                    "\n── rev {} · {}{conv_tail} ──\nsummary: {}\n\n{}\n",
                    r.rev,
                    fmt_ts(r.ts as u64),
                    r.summary,
                    r.content
                ));
                if !r.notes.is_empty() {
                    out.push_str(&format!("\nnotes: {}\n", r.notes));
                }
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionListArgs {
    /// Max solutions to show (default 25, capped at 200).
    #[serde(default)]
    max: Option<u32>,
}

pub struct SolutionList;
impl Skill for SolutionList {
    fn name(&self) -> &'static str {
        "solution_list"
    }
    fn description(&self) -> &'static str {
        "List recorded solutions (id, problem, rev count, last updated, tags), most recently updated first."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionListArgs>()?;
            let mem = &server.memory;
            let max = args.max.unwrap_or(25).clamp(1, 200) as i64;
            let rows: Vec<SolutionRow> = sqlx::query_as(
                "SELECT id, problem, canon_key, concept_key, created_at, updated_at \
                 FROM solutions ORDER BY updated_at DESC LIMIT ?",
            )
            .bind(max)
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            if rows.is_empty() {
                return Ok(text_result("No recorded solutions.".to_string()));
            }
            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM solutions")
                .fetch_one(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            let mut out = format!(
                "{} solution{} ({} shown):\n",
                total.0,
                if total.0 == 1 { "" } else { "s" },
                rows.len()
            );
            for sol in &rows {
                let last: Option<(i64,)> =
                    sqlx::query_as("SELECT MAX(rev) FROM solution_revisions WHERE solution_id = ?")
                        .bind(&sol.id)
                        .fetch_optional(&mem.pool)
                        .await
                        .map_err(|e| internal(e.into()))?;
                let tags = load_solution_tags(&mem.pool, &sol.id).await?;
                let tag_line = if tags.is_empty() {
                    String::new()
                } else {
                    format!("    tags: {}\n", tags.join(", "))
                };
                out.push_str(&format!(
                    "\n  {}  rev {} · updated {}\n    {}\n{}",
                    sol.id,
                    last.map(|r| r.0).unwrap_or(0),
                    fmt_ts(sol.updated_at as u64),
                    truncate(&sol.problem.replace('\n', " "), 140),
                    tag_line,
                ));
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionUpdateArgs {
    /// Solution id to revise.
    id: String,
    /// New one-line summary for this revision.
    summary: String,
    /// New full content for this revision.
    content: String,
    /// Optional caveats / notes about what changed.
    #[serde(default)]
    notes: Option<String>,
    /// Optional replacement tag list. Omit to leave tags unchanged; pass `[]` to clear them.
    #[serde(default)]
    tags: Option<Vec<String>>,
}

pub struct SolutionUpdate;
impl Skill for SolutionUpdate {
    fn name(&self) -> &'static str {
        "solution_update"
    }
    fn description(&self) -> &'static str {
        "Append a new revision to an existing solution. Prior revisions are kept and visible via \
        solution_show — so the change history is preserved. Pass `tags` to replace the tag list \
        (use `[]` to clear); omit to leave tags unchanged."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionUpdateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionUpdateArgs>()?;
            let mem = &server.memory;
            let cap = mem.cfg.max_value_chars.max(1);
            if args.content.chars().count() > cap {
                return Err(invalid(format!(
                    "content too long: {} chars (max {cap})",
                    args.content.chars().count()
                )));
            }
            let mut tx = mem.pool.begin().await.map_err(|e| internal(e.into()))?;
            let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM solutions WHERE id = ?")
                .bind(&args.id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| internal(e.into()))?;
            if exists.is_none() {
                return Err(invalid(format!("no solution \"{}\"", args.id)));
            }
            let now = now_secs() as i64;
            let next_rev: (i64,) = sqlx::query_as(
                "SELECT COALESCE(MAX(rev), 0) + 1 FROM solution_revisions WHERE solution_id = ?",
            )
            .bind(&args.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            // Stamp the active conversation so traversal can answer
            // "which conversation produced rev N?"
            let conv_id = mem.current_conversation_id().await;
            sqlx::query(
                "INSERT INTO solution_revisions (solution_id, rev, ts, summary, content, notes, conversation_id) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&args.id)
            .bind(next_rev.0)
            .bind(now)
            .bind(&args.summary)
            .bind(&args.content)
            .bind(args.notes.unwrap_or_default())
            .bind(conv_id.as_deref())
            .execute(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            sqlx::query("UPDATE solutions SET updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(&args.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| internal(e.into()))?;
            if let Some(new_tags) = args.tags {
                let cleaned = clean_tags(new_tags);
                sqlx::query("DELETE FROM solution_tags WHERE solution_id = ?")
                    .bind(&args.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| internal(e.into()))?;
                for tag in &cleaned {
                    sqlx::query(
                        "INSERT INTO solution_tags (solution_id, tag, label) VALUES (?, ?, ?)",
                    )
                    .bind(&args.id)
                    .bind(tag.to_ascii_lowercase())
                    .bind(tag)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| internal(e.into()))?;
                }
            }
            tx.commit().await.map_err(|e| internal(e.into()))?;
            // Re-embed against the new summary so semantic recall stays
            // aligned with the current revision. The old embedding stays
            // intact if the call fails.
            if !mem.cfg.embedding_endpoint.trim().is_empty() {
                // Pull the solution's problem text to combine with the new summary.
                let problem: Option<(String,)> =
                    sqlx::query_as("SELECT problem FROM solutions WHERE id = ?")
                        .bind(&args.id)
                        .fetch_optional(&mem.pool)
                        .await
                        .ok()
                        .flatten();
                if let Some((problem,)) = problem {
                    let text = format!("{}\n\n{}", problem, args.summary);
                    if let Some(vec) = mem.embed(&server.http, &text).await {
                        let blob = embedding_to_blob(&vec);
                        let _ = sqlx::query("UPDATE solutions SET embedding = ? WHERE id = ?")
                            .bind(blob)
                            .bind(&args.id)
                            .execute(&mem.pool)
                            .await;
                    }
                }
            }
            Ok(text_result(format!(
                "Updated {} (now at rev {}) at {}.",
                args.id,
                next_rev.0,
                fmt_ts(now as u64)
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionForgetArgs {
    /// Solution id to delete (drops all revisions, tags, and links).
    id: String,
    /// Confirmation token returned by the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// Whitelist `solution_forget` for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct SolutionForget;
impl Skill for SolutionForget {
    fn name(&self) -> &'static str {
        "solution_forget"
    }
    fn description(&self) -> &'static str {
        "Delete a recorded solution and its full revision history. Destructive — first call \
        returns a confirm token; call again with confirm=<token> (or trust=true to whitelist for \
        the session). `[memory].allow_destructive=true` pre-authorizes. Also strips dangling \
        incoming links from other solutions."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionForgetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionForgetArgs>()?;
            let mem = &server.memory;
            let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM solutions WHERE id = ?")
                .bind(&args.id)
                .fetch_optional(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            if exists.is_none() {
                return Ok(text_result(format!(
                    "No solution \"{}\" — nothing to forget.",
                    args.id
                )));
            }
            if let Decision::Challenge(msg) = server.guard.check(
                &format!("solution_forget|{}", args.id),
                "solution_forget",
                mem.cfg.allow_destructive,
                &format!("delete solution {} (drops all revisions)", args.id),
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let mut tx = mem.pool.begin().await.map_err(|e| internal(e.into()))?;
            let dangling: (i64,) = sqlx::query_as(
                "SELECT COUNT(*) FROM solution_links WHERE to_id = ? AND from_id != ?",
            )
            .bind(&args.id)
            .bind(&args.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            sqlx::query("DELETE FROM solution_links WHERE to_id = ?")
                .bind(&args.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| internal(e.into()))?;
            // CASCADE removes this solution's own revisions, tags, and outbound links.
            sqlx::query("DELETE FROM solutions WHERE id = ?")
                .bind(&args.id)
                .execute(&mut *tx)
                .await
                .map_err(|e| internal(e.into()))?;
            tx.commit().await.map_err(|e| internal(e.into()))?;
            let cleaned = if dangling.0 > 0 {
                format!(
                    " Cleaned {} incoming link{} from other solution{}.",
                    dangling.0,
                    if dangling.0 == 1 { "" } else { "s" },
                    if dangling.0 == 1 { "" } else { "s" }
                )
            } else {
                String::new()
            };
            Ok(text_result(format!(
                "Forgot solution {}.{}",
                args.id, cleaned
            )))
        })
    }
}

// ---------------------------------------------------------------------------
// solution_link / unlink / graph / related
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionLinkArgs {
    /// Source solution id.
    from: String,
    /// Relation kind. Recommended: `supersedes`/`superseded-by`,
    /// `depends-on`/`dependency-of`, `alternative-to`, `related-to`, `see-also`.
    /// Any free-form kind is accepted; unknown kinds are treated as symmetric.
    kind: String,
    /// Target solution id.
    to: String,
    /// Optional note explaining the relation.
    #[serde(default)]
    note: Option<String>,
}

pub struct SolutionLink;
impl Skill for SolutionLink {
    fn name(&self) -> &'static str {
        "solution_link"
    }
    fn description(&self) -> &'static str {
        "Declare a typed relation FROM one solution TO another (e.g. supersedes, depends-on, \
        related-to, see-also). The reciprocal is added automatically on the target (supersedes \
        → superseded-by); free-form kinds are symmetric. Use solution_graph to walk these edges \
        and solution_related to rank by combined explicit + tag + concept signal."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionLinkArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionLinkArgs>()?;
            let mem = &server.memory;
            let kind = args.kind.trim().to_string();
            if kind.is_empty() {
                return Err(invalid("kind must not be empty"));
            }
            if args.from == args.to {
                return Err(invalid("from and to must be different solutions"));
            }
            let note = args.note.unwrap_or_default();
            let recip = reciprocal_kind(&kind);
            // Verify both solutions exist before opening the write transaction.
            for id in [&args.from, &args.to] {
                let r: Option<(String,)> = sqlx::query_as("SELECT id FROM solutions WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
                if r.is_none() {
                    return Err(invalid(format!("no solution \"{id}\"")));
                }
            }
            let mut tx = mem.pool.begin().await.map_err(|e| internal(e.into()))?;
            sqlx::query(
                "INSERT OR IGNORE INTO solution_links (from_id, kind, to_id, note) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&args.from)
            .bind(&kind)
            .bind(&args.to)
            .bind(&note)
            .execute(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            sqlx::query(
                "INSERT OR IGNORE INTO solution_links (from_id, kind, to_id, note) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&args.to)
            .bind(&recip)
            .bind(&args.from)
            .bind(&note)
            .execute(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            let now = now_secs() as i64;
            sqlx::query("UPDATE solutions SET updated_at = ? WHERE id IN (?, ?)")
                .bind(now)
                .bind(&args.from)
                .bind(&args.to)
                .execute(&mut *tx)
                .await
                .map_err(|e| internal(e.into()))?;
            tx.commit().await.map_err(|e| internal(e.into()))?;
            Ok(text_result(format!(
                "Linked {} ─{kind}→ {} (reciprocal {} ─{recip}→ {} added).",
                args.from, args.to, args.to, args.from
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionUnlinkArgs {
    /// Source solution id (the side that originally held the link).
    from: String,
    /// Kind of the link to remove.
    kind: String,
    /// Target solution id.
    to: String,
}

pub struct SolutionUnlink;
impl Skill for SolutionUnlink {
    fn name(&self) -> &'static str {
        "solution_unlink"
    }
    fn description(&self) -> &'static str {
        "Remove a typed link from one solution to another. The reciprocal link on the target is \
        also removed automatically. The solutions themselves stay."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionUnlinkArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionUnlinkArgs>()?;
            let mem = &server.memory;
            let recip = reciprocal_kind(&args.kind);
            let mut tx = mem.pool.begin().await.map_err(|e| internal(e.into()))?;
            let r1 = sqlx::query(
                "DELETE FROM solution_links WHERE from_id = ? AND kind = ? AND to_id = ?",
            )
            .bind(&args.from)
            .bind(&args.kind)
            .bind(&args.to)
            .execute(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            let r2 = sqlx::query(
                "DELETE FROM solution_links WHERE from_id = ? AND kind = ? AND to_id = ?",
            )
            .bind(&args.to)
            .bind(&recip)
            .bind(&args.from)
            .execute(&mut *tx)
            .await
            .map_err(|e| internal(e.into()))?;
            tx.commit().await.map_err(|e| internal(e.into()))?;
            if r1.rows_affected() == 0 && r2.rows_affected() == 0 {
                return Ok(text_result(format!(
                    "No link {} ─{}→ {} was present; nothing to remove.",
                    args.from, args.kind, args.to
                )));
            }
            Ok(text_result(format!(
                "Unlinked {} ─{}→ {} (and removed the reciprocal {} ─{}→ {}).",
                args.from, args.kind, args.to, args.to, recip, args.from
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionGraphArgs {
    /// Solution id at the center of the subgraph.
    id: String,
    /// How many hops to walk outward (default 2, max 5).
    #[serde(default)]
    depth: Option<u32>,
}

pub struct SolutionGraph;
impl Skill for SolutionGraph {
    fn name(&self) -> &'static str {
        "solution_graph"
    }
    fn description(&self) -> &'static str {
        "Render the EXPLICIT-link subgraph around one solution: BFS outward to `depth` hops \
        (default 2, max 5), showing typed edges (supersedes, depends-on, related-to, …) to \
        every reachable solution. Use solution_related for an implicit-similarity ranking that \
        also weighs shared tags and concept-token overlap."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionGraphArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionGraphArgs>()?;
            let mem = &server.memory;
            let depth = args.depth.unwrap_or(2).min(5);
            let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM solutions WHERE id = ?")
                .bind(&args.id)
                .fetch_optional(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            if exists.is_none() {
                return Err(invalid(format!("no solution \"{}\"", args.id)));
            }
            let all_links: Vec<(String, String, String)> =
                sqlx::query_as("SELECT from_id, kind, to_id FROM solution_links")
                    .fetch_all(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            let mut out_edges: HashMap<String, Vec<(String, String)>> = HashMap::new();
            for (from, kind, to) in all_links {
                out_edges.entry(from).or_default().push((kind, to));
            }
            let mut seen: HashSet<String> = HashSet::new();
            let mut layers: Vec<Vec<(String, String, String)>> = Vec::new();
            seen.insert(args.id.clone());
            layers.push(vec![(args.id.clone(), String::new(), String::new())]);
            for _ in 0..depth {
                let mut next_layer: Vec<(String, String, String)> = Vec::new();
                let prev = layers.last().unwrap();
                for (node, _, _) in prev {
                    if let Some(edges) = out_edges.get(node) {
                        for (kind, to) in edges {
                            if seen.insert(to.clone()) {
                                next_layer.push((to.clone(), kind.clone(), node.clone()));
                            }
                        }
                    }
                }
                if next_layer.is_empty() {
                    break;
                }
                layers.push(next_layer);
            }
            let visited_ids: Vec<String> = seen.iter().cloned().collect();
            let mut problems: HashMap<String, String> = HashMap::new();
            for id in &visited_ids {
                let p: Option<(String,)> =
                    sqlx::query_as("SELECT problem FROM solutions WHERE id = ?")
                        .bind(id)
                        .fetch_optional(&mem.pool)
                        .await
                        .map_err(|e| internal(e.into()))?;
                if let Some(p) = p {
                    problems.insert(id.clone(), p.0);
                }
            }
            let mut out = format!("Graph around {} (depth {}):\n", args.id, depth);
            for (d, layer) in layers.iter().enumerate() {
                for (node, kind, parent) in layer {
                    let problem = problems
                        .get(node)
                        .map(|p| truncate(&p.replace('\n', " "), 100))
                        .unwrap_or_else(|| "?".into());
                    let indent = "  ".repeat(d);
                    if d == 0 {
                        out.push_str(&format!("{indent}{node}: {problem}\n"));
                    } else {
                        out.push_str(&format!(
                            "{indent}─{kind}→ {node} (from {parent}): {problem}\n"
                        ));
                    }
                }
            }
            let reachable = seen.len() - 1;
            out.push_str(&format!("\n{} solution(s) reachable.\n", reachable));
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionRelatedArgs {
    /// Solution id to find neighbors for.
    id: String,
    /// Max related solutions to return (default 5, capped at 20).
    #[serde(default)]
    max: Option<u32>,
}

pub struct SolutionRelated;
impl Skill for SolutionRelated {
    fn name(&self) -> &'static str {
        "solution_related"
    }
    fn description(&self) -> &'static str {
        "Rank solutions related to one source, combining EXPLICIT links (weight 30 per link) + \
        shared TAGS (2 per tag) + concept-token JACCARD overlap (20 Ã— overlap). Returns the top \
        `max` as advisory suggestions, with the contributing signals shown."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionRelatedArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionRelatedArgs>()?;
            let mem = &server.memory;
            let max = args.max.unwrap_or(5).clamp(1, 20) as usize;
            let src: Option<SolutionRow> = sqlx::query_as(
                "SELECT id, problem, canon_key, concept_key, created_at, updated_at \
                 FROM solutions WHERE id = ?",
            )
            .bind(&args.id)
            .fetch_optional(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let Some(src) = src else {
                return Err(invalid(format!("no solution \"{}\"", args.id)));
            };
            let src_tags = load_solution_tags(&mem.pool, &src.id).await?;
            let src_tag_set: HashSet<String> =
                src_tags.iter().map(|t| t.to_ascii_lowercase()).collect();
            let src_tokens: HashSet<String> = src
                .concept_key
                .as_deref()
                .map(|k| k.split_whitespace().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            let outgoing_rows: Vec<(String, String)> =
                sqlx::query_as("SELECT to_id, kind FROM solution_links WHERE from_id = ?")
                    .bind(&src.id)
                    .fetch_all(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
            for (to, kind) in outgoing_rows {
                outgoing.entry(to).or_default().push(kind);
            }
            let all_others: Vec<SolutionRow> = sqlx::query_as(
                "SELECT id, problem, canon_key, concept_key, created_at, updated_at \
                 FROM solutions WHERE id != ?",
            )
            .bind(&src.id)
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            // Pull all tags once and group by solution_id.
            let all_tags: Vec<(String, String)> =
                sqlx::query_as("SELECT solution_id, tag FROM solution_tags")
                    .fetch_all(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            let mut tags_by_sol: HashMap<String, HashSet<String>> = HashMap::new();
            for (sid, tag) in all_tags {
                tags_by_sol.entry(sid).or_default().insert(tag);
            }
            #[derive(Default)]
            struct Score {
                total: f64,
                signals: Vec<String>,
            }
            let mut scored: Vec<(SolutionRow, Score)> = Vec::new();
            let empty_set: HashSet<String> = HashSet::new();
            for sol in all_others {
                let mut s = Score::default();
                if let Some(kinds) = outgoing.get(&sol.id) {
                    s.total += 30.0 * kinds.len() as f64;
                    s.signals
                        .push(format!("explicit link: {}", kinds.join(", ")));
                }
                let other_tags = tags_by_sol.get(&sol.id).unwrap_or(&empty_set);
                let tag_overlap = other_tags
                    .iter()
                    .filter(|t| src_tag_set.contains(*t))
                    .count();
                if tag_overlap > 0 {
                    s.total += 2.0 * tag_overlap as f64;
                    s.signals.push(format!("{tag_overlap} shared tag(s)"));
                }
                if !src_tokens.is_empty() {
                    if let Some(other_key) = &sol.concept_key {
                        let other: HashSet<&str> = other_key.split_whitespace().collect();
                        let src_refs: HashSet<&str> =
                            src_tokens.iter().map(|s| s.as_str()).collect();
                        let inter = other.intersection(&src_refs).count();
                        if inter > 0 {
                            let union = other.union(&src_refs).count().max(1);
                            let jaccard = inter as f64 / union as f64;
                            s.total += 20.0 * jaccard;
                            s.signals.push(format!("concept overlap {:.2}", jaccard));
                        }
                    }
                }
                if s.total > 0.0 {
                    scored.push((sol, s));
                }
            }
            if scored.is_empty() {
                return Ok(text_result(format!(
                    "No related solutions found for {}.",
                    args.id
                )));
            }
            scored.sort_by(|a, b| {
                b.1.total
                    .partial_cmp(&a.1.total)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.0.updated_at.cmp(&a.0.updated_at))
            });
            let total = scored.len();
            let mut out = format!(
                "{} related solution{} to {} (advisory — verify before reusing):\n",
                total,
                if total == 1 { "" } else { "s" },
                args.id
            );
            for (i, (sol, s)) in scored.iter().take(max).enumerate() {
                out.push_str(&format!(
                    "\n{}. {}  (score {:.1}, updated {})\n   Problem: {}\n   Signals: {}\n",
                    i + 1,
                    sol.id,
                    s.total,
                    fmt_ts(sol.updated_at as u64),
                    truncate(&sol.problem.replace('\n', " "), 140),
                    s.signals.join(" · "),
                ));
            }
            Ok(text_result(out))
        })
    }
}

// ---------------------------------------------------------------------------
// solution_alias_add / solution_alias_remove — multiple phrasings per
// solution. Closes the "we'll never recall this if someone asks it
// differently" gap: every alias contributes its own canon_key / concept_key
// (for token-overlap scoring) and its own embedding (for semantic scoring).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionAliasAddArgs {
    /// Solution id this alias attaches to.
    id: String,
    /// A phrasing of the same underlying question — what someone might have
    /// asked instead. E.g. for the Seattle-Redmond distance solution:
    /// "How far is it from downtown Seattle to Microsoft HQ?"
    phrasing: String,
}

pub struct SolutionAliasAdd;
impl Skill for SolutionAliasAdd {
    fn name(&self) -> &'static str {
        "solution_alias_add"
    }
    fn description(&self) -> &'static str {
        "Attach an alternate PHRASING of the same underlying question to a recorded solution. \
        Recall scoring (both token-overlap and semantic) considers every phrasing — so a \
        question worded differently from the original problem text still surfaces the \
        solution. Use when you notice the same solution would apply to a question asked in \
        a way the original problem text wouldn't match."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionAliasAddArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionAliasAddArgs>()?;
            let mem = &server.memory;
            let phrasing = args.phrasing.trim().to_string();
            if phrasing.is_empty() {
                return Err(invalid("phrasing must not be empty".to_string()));
            }
            let exists: Option<(String,)> = sqlx::query_as("SELECT id FROM solutions WHERE id = ?")
                .bind(&args.id)
                .fetch_optional(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            if exists.is_none() {
                return Err(invalid(format!("no solution \"{}\"", args.id)));
            }
            let canon = crate::provider::canonical_query(&phrasing);
            let concept = concept_key_of(&phrasing);
            // Cheap hash so we can de-dupe identical phrasings without
            // building an index on a long TEXT column.
            let hash = phrasing_hash(&canon);
            let now = now_secs() as i64;
            sqlx::query(
                "INSERT OR IGNORE INTO solution_phrasings \
                 (solution_id, hash, phrasing, canon_key, concept_key, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&args.id)
            .bind(&hash)
            .bind(&phrasing)
            .bind(&canon)
            .bind(&concept)
            .bind(now)
            .execute(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            // Best-effort embed of the alias for semantic recall.
            if !mem.cfg.embedding_endpoint.trim().is_empty() {
                if let Some(vec) = mem.embed(&server.http, &phrasing).await {
                    let blob = embedding_to_blob(&vec);
                    let _ = sqlx::query(
                        "UPDATE solution_phrasings SET embedding = ? \
                         WHERE solution_id = ? AND hash = ?",
                    )
                    .bind(blob)
                    .bind(&args.id)
                    .bind(&hash)
                    .execute(&mem.pool)
                    .await;
                }
            }
            Ok(text_result(format!(
                "Attached phrasing to {}. Future recall now considers it.",
                args.id
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionAliasRemoveArgs {
    /// Solution id the alias is attached to.
    id: String,
    /// The exact phrasing to remove (must match a previously-added one).
    phrasing: String,
}

pub struct SolutionAliasRemove;
impl Skill for SolutionAliasRemove {
    fn name(&self) -> &'static str {
        "solution_alias_remove"
    }
    fn description(&self) -> &'static str {
        "Detach a previously-added alternate phrasing from a solution. Match is by canonical \
        form of the phrasing (case / stop-word insensitive)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionAliasRemoveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionAliasRemoveArgs>()?;
            let mem = &server.memory;
            let canon = crate::provider::canonical_query(args.phrasing.trim());
            let hash = phrasing_hash(&canon);
            let r =
                sqlx::query("DELETE FROM solution_phrasings WHERE solution_id = ? AND hash = ?")
                    .bind(&args.id)
                    .bind(&hash)
                    .execute(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            if r.rows_affected() == 0 {
                Ok(text_result(format!(
                    "No matching phrasing on {} — nothing to detach.",
                    args.id
                )))
            } else {
                Ok(text_result(format!("Detached phrasing from {}.", args.id)))
            }
        })
    }
}

/// Stable 64-bit hash of a canonical phrasing (FNV-1a). Used as the
/// dedup key in `solution_phrasings` so we don't index on the full TEXT.
fn phrasing_hash(canon: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in canon.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

// ---------------------------------------------------------------------------
// synonym_* tools — learned single-token aliases
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SynonymAddArgs {
    /// The token to alias (lowercased automatically).
    token: String,
    /// What to fold the token to (lowercased automatically).
    canonical: String,
    /// Optional note ("learned 2026-05-30 from the k8s docs", etc.).
    #[serde(default)]
    note: Option<String>,
}

pub struct SynonymAdd;
impl Skill for SynonymAdd {
    fn name(&self) -> &'static str {
        "synonym_add"
    }
    fn description(&self) -> &'static str {
        "Teach the server a single-token synonym: occurrences of `token` are folded to \
        `canonical` everywhere queries are normalized — for both the search cache AND the \
        memory/solution recall. Persisted across restarts. Use this to absorb domain \
        terminology (e.g. token=\"k8s\", canonical=\"kubernetes\") as you learn it; the system \
        ships with NO built-in synonyms."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SynonymAddArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SynonymAddArgs>()?;
            let mem = &server.memory;
            let token = args.token.trim().to_ascii_lowercase();
            let canonical = args.canonical.trim().to_ascii_lowercase();
            if token.is_empty() || canonical.is_empty() {
                return Err(invalid("token and canonical must be non-empty"));
            }
            if token == canonical {
                return Err(invalid(
                    "token equals canonical — that fold would be a no-op",
                ));
            }
            if token.contains(char::is_whitespace) || canonical.contains(char::is_whitespace) {
                return Err(invalid(
                    "synonyms are single tokens only — no whitespace allowed",
                ));
            }
            let note = args.note.unwrap_or_default();
            let now = now_secs() as i64;
            sqlx::query(
                "INSERT INTO synonyms (token, canonical, note, created_at) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT(token) DO UPDATE SET canonical = excluded.canonical, \
                     note = excluded.note",
            )
            .bind(&token)
            .bind(&canonical)
            .bind(&note)
            .bind(now)
            .execute(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            if let Ok(mut map) = mem.synonyms.write() {
                map.insert(token.clone(), canonical.clone());
            }
            Ok(text_result(format!(
                "Synonym learned: {token} → {canonical}."
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SynonymRemoveArgs {
    /// The token whose alias should be removed.
    token: String,
}

pub struct SynonymRemove;
impl Skill for SynonymRemove {
    fn name(&self) -> &'static str {
        "synonym_remove"
    }
    fn description(&self) -> &'static str {
        "Remove a learned synonym so the token stops folding to its canonical form."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SynonymRemoveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SynonymRemoveArgs>()?;
            let mem = &server.memory;
            let token = args.token.trim().to_ascii_lowercase();
            let r = sqlx::query("DELETE FROM synonyms WHERE token = ?")
                .bind(&token)
                .execute(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            if r.rows_affected() == 0 {
                return Ok(text_result(format!(
                    "No synonym for token=\"{token}\" — nothing to remove."
                )));
            }
            if let Ok(mut map) = mem.synonyms.write() {
                map.remove(&token);
            }
            Ok(text_result(format!(
                "Removed synonym for token=\"{token}\"."
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SynonymListArgs {
    /// Max synonyms to show (default 50, capped at 500).
    #[serde(default)]
    max: Option<u32>,
}

pub struct SynonymList;
impl Skill for SynonymList {
    fn name(&self) -> &'static str {
        "synonym_list"
    }
    fn description(&self) -> &'static str {
        "List all learned synonyms (token → canonical) with any attached notes, newest first."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SynonymListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SynonymListArgs>()?;
            let mem = &server.memory;
            let max = args.max.unwrap_or(50).clamp(1, 500) as i64;
            let rows: Vec<(String, String, String, i64)> = sqlx::query_as(
                "SELECT token, canonical, note, created_at FROM synonyms \
                 ORDER BY created_at DESC LIMIT ?",
            )
            .bind(max)
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            if rows.is_empty() {
                return Ok(text_result(
                    "No synonyms learned yet. Add one with synonym_add { token, canonical }."
                        .to_string(),
                ));
            }
            let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM synonyms")
                .fetch_one(&mem.pool)
                .await
                .map_err(|e| internal(e.into()))?;
            let mut out = format!(
                "{} synonym{} ({} shown):\n",
                total.0,
                if total.0 == 1 { "" } else { "s" },
                rows.len()
            );
            for (token, canonical, note, _ts) in rows {
                let note_tail = if note.is_empty() {
                    String::new()
                } else {
                    format!("  ({note})")
                };
                out.push_str(&format!("  {token} → {canonical}{note_tail}\n"));
            }
            Ok(text_result(out))
        })
    }
}

// ---------------------------------------------------------------------------
// Conversations — read-only traversal (`conversation_list` /
// `conversation_show` / `solution_conversations`) plus destructive cleanup
// (`conversation_forget` / `conversation_prune`, defined further below).
// Conversation rows are written by the dispatch wrapper and by
// `solution_record` / `solution_update`.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConversationListArgs {
    /// Max conversations to return, most recently active first. Default 20, capped at 200.
    #[serde(default)]
    max: Option<u32>,
}

#[derive(FromRow)]
struct ConversationRow {
    id: String,
    started_at: i64,
    last_seen_at: i64,
    turn_count: i64,
    first_query: Option<String>,
}

pub struct ConversationList;
impl Skill for ConversationList {
    fn name(&self) -> &'static str {
        "conversation_list"
    }
    fn description(&self) -> &'static str {
        "List recorded conversations, most recently active first. Each row shows the id (use it \
        with conversation_show), turn count, started/last-seen times, and the first query seen. \
        Conversations are bounded by an idle gap (a long pause ends one and starts the next)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConversationListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ConversationListArgs>()?;
            let mem = &server.memory;
            let limit = args.max.unwrap_or(20).clamp(1, 200) as i64;
            let rows: Vec<ConversationRow> = sqlx::query_as(
                "SELECT id, started_at, last_seen_at, turn_count, first_query FROM conversations \
                 ORDER BY last_seen_at DESC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            if rows.is_empty() {
                return Ok(text_result("No recorded conversations yet.".to_string()));
            }
            let mut out = format!(
                "{} conversation{}:\n",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" }
            );
            for r in &rows {
                let preview = r
                    .first_query
                    .as_deref()
                    .map(|q| {
                        let q: String = q.replace('\n', " ").chars().take(80).collect();
                        format!(" — {q}")
                    })
                    .unwrap_or_default();
                out.push_str(&format!(
                    "  • {} · {} turn{} · {} → {}{preview}\n",
                    r.id,
                    r.turn_count,
                    if r.turn_count == 1 { "" } else { "s" },
                    fmt_ts(r.started_at as u64),
                    fmt_ts(r.last_seen_at as u64),
                ));
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConversationShowArgs {
    /// Conversation id (from `conversation_list` or a `solution_show` revision line).
    id: String,
    /// Max turns to return (oldest first). Default 100, capped at 1000.
    #[serde(default)]
    max: Option<u32>,
}

#[derive(FromRow)]
struct TurnRow {
    seq: i64,
    ts: i64,
    tool_name: String,
    query: Option<String>,
    response_excerpt: String,
}

pub struct ConversationShow;
impl Skill for ConversationShow {
    fn name(&self) -> &'static str {
        "conversation_show"
    }
    fn description(&self) -> &'static str {
        "Walk a conversation: every tool call in order, with query and a short response excerpt. \
        Use this after solution_find / the recall preamble to see WHAT ELSE happened around the \
        time a solution was recorded — adjacent searches, retrievals, and related work."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConversationShowArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ConversationShowArgs>()?;
            let mem = &server.memory;
            let limit = args.max.unwrap_or(100).clamp(1, 1000) as i64;
            let conv: Option<ConversationRow> = sqlx::query_as(
                "SELECT id, started_at, last_seen_at, turn_count, first_query \
                 FROM conversations WHERE id = ?",
            )
            .bind(&args.id)
            .fetch_optional(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let Some(conv) = conv else {
                return Err(invalid(format!("no conversation \"{}\"", args.id)));
            };
            let turns: Vec<TurnRow> = sqlx::query_as(
                "SELECT seq, ts, tool_name, query, response_excerpt FROM conversation_turns \
                 WHERE conversation_id = ? ORDER BY seq ASC LIMIT ?",
            )
            .bind(&args.id)
            .bind(limit)
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            let sols: Vec<(String, i64)> = sqlx::query_as(
                "SELECT solution_id, rev FROM solution_revisions \
                 WHERE conversation_id = ? ORDER BY ts ASC",
            )
            .bind(&args.id)
            .fetch_all(&mem.pool)
            .await
            .unwrap_or_default();
            let mut out = format!(
                "{} · {} turn{} · {} → {}\n",
                conv.id,
                conv.turn_count,
                if conv.turn_count == 1 { "" } else { "s" },
                fmt_ts(conv.started_at as u64),
                fmt_ts(conv.last_seen_at as u64),
            );
            if !sols.is_empty() {
                out.push_str("Solutions touched in this conversation:\n");
                for (sid, rev) in &sols {
                    out.push_str(&format!("  • {sid} (rev {rev})\n"));
                }
            }
            out.push_str("\nTurns:\n");
            for t in &turns {
                let q = t.query.as_deref().unwrap_or("");
                let qline = if q.is_empty() {
                    String::new()
                } else {
                    let q: String = q.replace('\n', " ").chars().take(160).collect();
                    format!(" — query: {q}")
                };
                let excerpt: String = t
                    .response_excerpt
                    .replace('\n', " ")
                    .chars()
                    .take(160)
                    .collect();
                out.push_str(&format!(
                    "  [{:>3}] {} · {}{qline}\n",
                    t.seq,
                    fmt_ts(t.ts as u64),
                    t.tool_name
                ));
                if !excerpt.is_empty() {
                    out.push_str(&format!("        ↳ {excerpt}\n"));
                }
            }
            if (turns.len() as i64) >= limit && conv.turn_count > limit {
                out.push_str(&format!(
                    "\n(showing first {limit} of {} turns — pass max= to see more)\n",
                    conv.turn_count
                ));
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SolutionConversationsArgs {
    /// Solution id (e.g. `sol-3`).
    id: String,
}

pub struct SolutionConversations;
impl Skill for SolutionConversations {
    fn name(&self) -> &'static str {
        "solution_conversations"
    }
    fn description(&self) -> &'static str {
        "List the conversations that contributed revisions to a recorded solution — answering \
        \"what conversation was this solution a part of?\" Each row gives the conversation id (use \
        conversation_show next), which revisions it produced, and when. Solutions updated across \
        sessions will list multiple conversations."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SolutionConversationsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SolutionConversationsArgs>()?;
            let mem = &server.memory;
            let rows: Vec<(Option<String>, i64, i64)> = sqlx::query_as(
                "SELECT conversation_id, rev, ts FROM solution_revisions \
                 WHERE solution_id = ? ORDER BY rev ASC",
            )
            .bind(&args.id)
            .fetch_all(&mem.pool)
            .await
            .map_err(|e| internal(e.into()))?;
            if rows.is_empty() {
                return Err(invalid(format!("no solution \"{}\"", args.id)));
            }
            // Group rev numbers by conversation_id, preserving first-seen order.
            let mut order: Vec<String> = Vec::new();
            let mut by_conv: HashMap<String, (Vec<i64>, i64)> = HashMap::new();
            let mut unknown: Vec<i64> = Vec::new();
            for (conv, rev, ts) in rows {
                match conv {
                    Some(c) => {
                        let entry = by_conv.entry(c.clone()).or_insert_with(|| (Vec::new(), ts));
                        entry.0.push(rev);
                        if !order.contains(&c) {
                            order.push(c);
                        }
                    }
                    None => unknown.push(rev),
                }
            }
            let mut out = format!("Conversations for {}:\n", args.id);
            if order.is_empty() && !unknown.is_empty() {
                out.push_str("  (no conversation context recorded — predates conversation tracking or memory was off)\n");
            }
            for c in &order {
                let (revs, ts) = by_conv.get(c).unwrap();
                let revs_str = revs
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "  • {c} — rev{} {revs_str} · {}\n",
                    if revs.len() == 1 { "" } else { "s" },
                    fmt_ts(*ts as u64)
                ));
            }
            if !unknown.is_empty() {
                let revs_str = unknown
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!(
                    "  • (no conversation) — rev{} {revs_str}\n",
                    if unknown.len() == 1 { "" } else { "s" }
                ));
            }
            out.push_str("\n↳ conversation_show id=\"<id>\" to walk the surrounding turns.\n");
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConversationForgetArgs {
    /// Conversation id to delete (drops all turns; nulls back-pointers from
    /// `solution_revisions` so revisions stay queryable).
    id: String,
    /// Confirmation token returned by the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// Whitelist `conversation_forget` for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct ConversationForget;
impl Skill for ConversationForget {
    fn name(&self) -> &'static str {
        "conversation_forget"
    }
    fn description(&self) -> &'static str {
        "Delete one recorded conversation and its turns. Destructive — first call returns a \
        confirm token; call again with confirm=<token> (or trust=true to whitelist for the \
        session). `[memory].allow_destructive=true` pre-authorizes. Revisions of solutions that \
        referenced this conversation keep their content; their conversation_id is set to NULL."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConversationForgetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ConversationForgetArgs>()?;
            let mem = &server.memory;
            let exists: Option<(String,)> =
                sqlx::query_as("SELECT id FROM conversations WHERE id = ?")
                    .bind(&args.id)
                    .fetch_optional(&mem.pool)
                    .await
                    .map_err(|e| internal(e.into()))?;
            if exists.is_none() {
                return Ok(text_result(format!(
                    "No conversation \"{}\" — nothing to forget.",
                    args.id
                )));
            }
            if let Decision::Challenge(msg) = server.guard.check(
                &format!("conversation_forget|{}", args.id),
                "conversation_forget",
                mem.cfg.allow_destructive,
                &format!("delete conversation {} (drops all turns)", args.id),
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            mem.forget_conversation(&args.id)
                .await
                .map_err(|e| internal(e.into()))?;
            Ok(text_result(format!("Forgot conversation {}.", args.id)))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConversationPruneArgs {
    /// Delete conversations whose `last_seen_at` is older than this many days.
    /// 0 or omitted = no age-based pruning.
    #[serde(default)]
    older_than_days: Option<u32>,
    /// Keep only the N most recently active conversations; delete the rest.
    /// 0 or omitted = no count-based pruning.
    #[serde(default)]
    keep_newest: Option<u32>,
    /// Preview only — report how many conversations WOULD be deleted, don't
    /// touch anything. Bypasses the confirm-token guard. Default false.
    #[serde(default)]
    dry_run: Option<bool>,
    /// Confirmation token returned by the first non-dry-run call.
    #[serde(default)]
    confirm: Option<String>,
    /// Whitelist `conversation_prune` for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct ConversationPrune;
impl Skill for ConversationPrune {
    fn name(&self) -> &'static str {
        "conversation_prune"
    }
    fn description(&self) -> &'static str {
        "Bulk-delete conversations by retention policy. Filters: `older_than_days` and/or \
        `keep_newest`. When neither is set, falls back to the configured retention \
        (`[memory].conversation_retention_days`, `[memory].max_conversations`). \
        Use `dry_run=true` first to preview the count without deleting. Destructive when \
        live — first non-dry-run call returns a confirm token; `[memory].allow_destructive=true` \
        pre-authorizes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConversationPruneArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ConversationPruneArgs>()?;
            let mem = &server.memory;
            let dry = args.dry_run.unwrap_or(false);
            // Fall back to configured policy when neither knob is set.
            let days = args
                .older_than_days
                .unwrap_or(mem.cfg.conversation_retention_days);
            let keep = args
                .keep_newest
                .map(|n| n as usize)
                .unwrap_or(mem.cfg.max_conversations);
            if days == 0 && keep == 0 {
                return Ok(text_result(
                    "No retention policy specified and none configured — nothing to prune. \
                     Pass older_than_days and/or keep_newest, or set them in [memory]."
                        .to_string(),
                ));
            }
            // Preview-first: always cheap, lets the caller see the impact.
            let would = mem
                .prune_conversations(days, keep, true)
                .await
                .map_err(|e| internal(e.into()))?;
            if dry {
                return Ok(text_result(format!(
                    "[dry run] would delete {would} conversation{} \
                     (older_than_days={days}, keep_newest={keep})",
                    if would == 1 { "" } else { "s" }
                )));
            }
            if would == 0 {
                return Ok(text_result(
                    "Nothing to prune under that policy.".to_string(),
                ));
            }
            if let Decision::Challenge(msg) = server.guard.check(
                &format!("conversation_prune|{days}|{keep}"),
                "conversation_prune",
                mem.cfg.allow_destructive,
                &format!(
                    "delete {would} conversation{} (older_than_days={days}, keep_newest={keep})",
                    if would == 1 { "" } else { "s" }
                ),
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let deleted = mem
                .prune_conversations(days, keep, false)
                .await
                .map_err(|e| internal(e.into()))?;
            Ok(text_result(format!(
                "Pruned {deleted} conversation{}.",
                if deleted == 1 { "" } else { "s" }
            )))
        })
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(MemorySave),
        Box::new(MemoryGet),
        Box::new(MemoryList),
        Box::new(MemorySearch),
        Box::new(MemoryForget),
        Box::new(SolutionRecord),
        Box::new(SolutionFind),
        Box::new(SolutionShow),
        Box::new(SolutionList),
        Box::new(SolutionUpdate),
        Box::new(SolutionForget),
        Box::new(SolutionLink),
        Box::new(SolutionUnlink),
        Box::new(SolutionGraph),
        Box::new(SolutionRelated),
        Box::new(SolutionAliasAdd),
        Box::new(SolutionAliasRemove),
        Box::new(SynonymAdd),
        Box::new(SynonymRemove),
        Box::new(SynonymList),
        Box::new(ConversationList),
        Box::new(ConversationShow),
        Box::new(SolutionConversations),
        Box::new(ConversationForget),
        Box::new(ConversationPrune),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    async fn fresh_memory() -> Memory {
        fresh_memory_with(|_| {}).await
    }

    /// Build a fresh `Memory` with a unique tempdir and let the caller tweak
    /// the config (e.g. disable conversation recording for that test).
    async fn fresh_memory_with(tweak: impl FnOnce(&mut config::Memory)) -> Memory {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("lodestone-memory-test-{n}-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let mut cfg = config::Memory {
            enabled: true,
            dir: dir.to_string_lossy().to_string(),
            max_entries: 1000,
            max_value_chars: 1_000_000,
            ..Default::default()
        };
        tweak(&mut cfg);
        Memory::new(cfg).await.unwrap()
    }

    #[tokio::test]
    async fn schema_and_version_recorded_on_fresh_db() {
        let mem = fresh_memory().await;
        // Every table from v1 + v2 must exist.
        for table in [
            "memory",
            "solutions",
            "solution_revisions",
            "solution_tags",
            "solution_links",
            "synonyms",
            "conversations",
            "conversation_turns",
            "solution_phrasings",
            "_schema_version",
        ] {
            let q = format!("SELECT COUNT(*) FROM {table}");
            let _: (i64,) = sqlx::query_as(&q).fetch_one(&mem.pool).await.unwrap();
        }
        // Both migrations applied.
        let v: (i64,) = sqlx::query_as("SELECT MAX(version) FROM _schema_version")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(v.0, 3, "migrations v1 through v3 should all be recorded");
    }

    #[tokio::test]
    async fn rerunning_migrations_is_idempotent() {
        let mem = fresh_memory().await;
        // Apply again on the same connection — must not error or duplicate.
        apply_migrations(&mem.pool).await.unwrap();
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _schema_version")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(
            count.0, 3,
            "migrations v1 through v3 should each be recorded exactly once"
        );
    }

    #[tokio::test]
    async fn synonym_add_writes_through_to_shared_map() {
        let mem = fresh_memory().await;
        sqlx::query(
            "INSERT INTO synonyms (token, canonical, note, created_at) VALUES (?, ?, '', ?)",
        )
        .bind("k8s")
        .bind("kubernetes")
        .bind(now_secs() as i64)
        .execute(&mem.pool)
        .await
        .unwrap();
        mem.synonyms
            .write()
            .unwrap()
            .insert("k8s".into(), "kubernetes".into());
        let with_alias = crate::provider::canonical_query("k8s deploy");
        let without_alias = crate::provider::canonical_query("kubernetes deploy");
        assert_eq!(with_alias, without_alias);
    }

    #[tokio::test]
    async fn reciprocal_kind_known_pairs_flip() {
        assert_eq!(reciprocal_kind("supersedes"), "superseded-by");
        assert_eq!(reciprocal_kind("depends-on"), "dependency-of");
        assert_eq!(reciprocal_kind("see-also"), "see-also");
        assert_eq!(reciprocal_kind("custom"), "custom");
    }

    #[tokio::test]
    async fn cascade_drops_revisions_and_tags() {
        let mem = fresh_memory().await;
        let now = now_secs() as i64;
        sqlx::query(
            "INSERT INTO solutions (id, problem, canon_key, created_at, updated_at) \
             VALUES ('sol-1', 'p', 'p', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO solution_revisions (solution_id, rev, ts, summary, content) \
             VALUES ('sol-1', 1, ?, 's', 'c')",
        )
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO solution_tags (solution_id, tag, label) VALUES ('sol-1','t','t')")
            .execute(&mem.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM solutions WHERE id = 'sol-1'")
            .execute(&mem.pool)
            .await
            .unwrap();
        let revs: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM solution_revisions")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        let tags: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM solution_tags")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(revs.0, 0, "CASCADE should drop revisions");
        assert_eq!(tags.0, 0, "CASCADE should drop tags");
    }

    /// Insert a no-revision solution row directly. Skips the
    /// record/update API so tests can hand-build a graph cheaply.
    async fn insert_sol(mem: &Memory, id: &str) {
        let now = now_secs() as i64;
        sqlx::query(
            "INSERT INTO solutions (id, problem, canon_key, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(format!("problem for {id}"))
        .bind(format!("canon-{id}"))
        .bind(now)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
    }

    async fn link(mem: &Memory, from: &str, kind: &str, to: &str) {
        sqlx::query("INSERT INTO solution_links (from_id, kind, to_id, note) VALUES (?, ?, ?, '')")
            .bind(from)
            .bind(kind)
            .bind(to)
            .execute(&mem.pool)
            .await
            .unwrap();
    }

    /// Walk a → b → c → d via `superseded-by` and confirm the head returned
    /// is `d`. This is the load-bearing property of supersession-aware
    /// recall: hand the model the *current* head, not a stale intermediate.
    #[tokio::test]
    async fn supersession_walk_returns_chain_head() {
        let mem = fresh_memory().await;
        for id in ["sol-a", "sol-b", "sol-c", "sol-d"] {
            insert_sol(&mem, id).await;
        }
        link(&mem, "sol-a", "superseded-by", "sol-b").await;
        link(&mem, "sol-b", "superseded-by", "sol-c").await;
        link(&mem, "sol-c", "superseded-by", "sol-d").await;
        assert_eq!(
            mem.walk_supersession_head("sol-a").await.as_deref(),
            Some("sol-d")
        );
        // The head itself has no outgoing supersession — None.
        assert_eq!(mem.walk_supersession_head("sol-d").await, None);
    }

    /// A `superseded-by` cycle (recorded incorrectly) must not lock the
    /// recall path into a loop. The walk terminates at the first repeat.
    #[tokio::test]
    async fn supersession_walk_terminates_on_cycle() {
        let mem = fresh_memory().await;
        for id in ["sol-x", "sol-y"] {
            insert_sol(&mem, id).await;
        }
        link(&mem, "sol-x", "superseded-by", "sol-y").await;
        link(&mem, "sol-y", "superseded-by", "sol-x").await;
        // Whichever stable id we land on, it's one of the two; what matters
        // is the call returns at all (no infinite loop, no panic).
        let head = mem.walk_supersession_head("sol-x").await;
        assert!(head.is_some());
        assert!(matches!(head.as_deref(), Some("sol-y") | Some("sol-x")));
    }

    /// First `current_conversation_id()` call mints a new id and writes a
    /// `conversations` row. Subsequent calls within the idle gap return the
    /// same id without inserting a new row.
    #[tokio::test]
    async fn conversation_id_is_stable_within_idle_gap() {
        let mem = fresh_memory().await;
        let a = mem.current_conversation_id().await.unwrap();
        let b = mem.current_conversation_id().await.unwrap();
        let c = mem.current_conversation_id().await.unwrap();
        assert_eq!(a, b);
        assert_eq!(b, c);
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "exactly one conversation row should exist");
    }

    /// When the in-memory `last_seen_secs` is rewound past the idle gap, the
    /// next call mints a fresh id and writes a second `conversations` row.
    #[tokio::test]
    async fn conversation_id_rotates_after_idle_gap() {
        let mem = fresh_memory().await;
        let first = mem.current_conversation_id().await.unwrap();
        // Forcibly age the active tracker beyond the gap.
        {
            let mut guard = mem.active_conv.lock().unwrap();
            if let Some(active) = guard.as_mut() {
                active.last_seen_secs = active
                    .last_seen_secs
                    .saturating_sub(mem.cfg.conversation_idle_gap_secs + 60);
            }
        }
        let second = mem.current_conversation_id().await.unwrap();
        assert_ne!(first, second);
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2, "two distinct conversations should be recorded");
    }

    /// `record_turn` writes one row per call, bumps `turn_count`, and stamps
    /// `first_query` on the first non-null query.
    #[tokio::test]
    async fn record_turn_appends_and_updates_metadata() {
        let mem = fresh_memory().await;
        let conv = mem.current_conversation_id().await.unwrap();
        mem.record_turn(&conv, "fs_read", None, "file contents")
            .await;
        mem.record_turn(&conv, "web_search", Some("first query"), "results...")
            .await;
        mem.record_turn(&conv, "wikipedia_search", Some("later query"), "more")
            .await;
        let row: ConversationRow = sqlx::query_as(
            "SELECT id, started_at, last_seen_at, turn_count, first_query FROM conversations WHERE id = ?",
        )
        .bind(&conv)
        .fetch_one(&mem.pool)
        .await
        .unwrap();
        assert_eq!(row.turn_count, 3);
        assert_eq!(row.first_query.as_deref(), Some("first query"));
        // Turns must be ordered by seq.
        let turns: Vec<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT seq, tool_name, query FROM conversation_turns \
             WHERE conversation_id = ? ORDER BY seq",
        )
        .bind(&conv)
        .fetch_all(&mem.pool)
        .await
        .unwrap();
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].1, "fs_read");
        assert_eq!(turns[0].2, None);
        assert_eq!(turns[1].1, "web_search");
        assert_eq!(turns[1].2.as_deref(), Some("first query"));
        assert_eq!(turns[2].1, "wikipedia_search");
        // Sequence must be strictly increasing.
        assert!(turns[0].0 < turns[1].0);
        assert!(turns[1].0 < turns[2].0);
    }

    /// In auto-recall (no explicit `tags=` filter), tag overlap with the
    /// query's concept tokens contributes to the score. Without this, single-
    /// noun queries can't clear the 30 threshold against richly-tagged
    /// solutions even when the tag is exactly the topic word in question.
    #[tokio::test]
    async fn auto_recall_counts_tag_overlap_with_query_tokens() {
        let mem = fresh_memory().await;
        // Record a solution with seattle/redmond tags, then query for "Seattle
        // Redmond distance" — the fuzzy path alone wouldn't fire because of
        // dilution, but tag overlap (2 tags) plus fuzzy should clear 30.
        let now = now_secs() as i64;
        sqlx::query(
            "INSERT INTO solutions (id, problem, canon_key, concept_key, created_at, updated_at) \
             VALUES ('sol-x', ?, ?, ?, ?, ?)",
        )
        .bind("Redmond approximately 40 miles east of Seattle - validate claim")
        .bind("redmond approximately 40 miles east seattle validate claim")
        .bind("redmond approximately 40 miles east seattle validate claim")
        .bind(now)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO solution_revisions \
             (solution_id, rev, ts, summary, content, notes, conversation_id) \
             VALUES ('sol-x', 1, ?, 's', 'c', '', NULL)",
        )
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        for tag in ["seattle", "redmond", "geography", "grid"] {
            sqlx::query(
                "INSERT INTO solution_tags (solution_id, tag, label) VALUES ('sol-x', ?, ?)",
            )
            .bind(tag)
            .bind(tag)
            .execute(&mem.pool)
            .await
            .unwrap();
        }
        let http = reqwest::Client::new();
        let hits = mem.auto_recall(&http, "Seattle Redmond distance", 5).await;
        assert!(
            !hits.is_empty(),
            "tag overlap (2) + fuzzy must clear the recall threshold"
        );
        assert_eq!(hits[0].id, "sol-x");
    }

    /// When `[memory].record_conversations = false`, the wrapper-side helper
    /// must short-circuit: no id is minted, no rows are written.
    #[tokio::test]
    async fn record_conversations_off_disables_tracking() {
        let mem = fresh_memory_with(|c| c.record_conversations = false).await;
        assert_eq!(mem.current_conversation_id().await, None);
        mem.record_turn("conv-x", "web_search", Some("q"), "result")
            .await;
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversation_turns")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    /// When `[memory].record_only_query_calls = true`, `record_turn` drops
    /// calls without a free-text query — keeps the log focused on intent.
    #[tokio::test]
    async fn record_only_query_calls_filters_silent_tools() {
        let mem = fresh_memory_with(|c| c.record_only_query_calls = true).await;
        let conv = mem.current_conversation_id().await.unwrap();
        mem.record_turn(&conv, "fs_read", None, "contents").await;
        mem.record_turn(&conv, "arithmetic_eval", Some("   "), "42")
            .await;
        mem.record_turn(&conv, "web_search", Some("rust ownership"), "results")
            .await;
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM conversation_turns WHERE conversation_id = ?")
                .bind(&conv)
                .fetch_one(&mem.pool)
                .await
                .unwrap();
        assert_eq!(count.0, 1, "only the query-bearing turn should be recorded");
    }

    /// `conversation_idle_gap_secs` is honored: a tiny gap means each call
    /// rotates to a new id.
    #[tokio::test]
    async fn idle_gap_config_governs_rotation() {
        // gap=0 means any subsequent call should rotate.
        let mem = fresh_memory_with(|c| c.conversation_idle_gap_secs = 0).await;
        let a = mem.current_conversation_id().await.unwrap();
        // Forcibly age the tracker by one second so the gap (0) is exceeded.
        {
            let mut guard = mem.active_conv.lock().unwrap();
            if let Some(active) = guard.as_mut() {
                active.last_seen_secs = active.last_seen_secs.saturating_sub(1);
            }
        }
        let b = mem.current_conversation_id().await.unwrap();
        assert_ne!(a, b, "with gap=0 a second call should mint a new id");
    }

    /// `forget_conversation` deletes the row, cascades to turns, and NULLs
    /// revisions' back-pointer (keeping revision content intact).
    #[tokio::test]
    async fn forget_conversation_preserves_revision_content() {
        let mem = fresh_memory().await;
        let conv = mem.current_conversation_id().await.unwrap();
        mem.record_turn(&conv, "web_search", Some("q"), "r").await;
        let now = now_secs() as i64;
        sqlx::query(
            "INSERT INTO solutions (id, problem, canon_key, created_at, updated_at) \
             VALUES ('sol-1', 'p', 'p', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO solution_revisions \
             (solution_id, rev, ts, summary, content, notes, conversation_id) \
             VALUES ('sol-1', 1, ?, 's', 'c', '', ?)",
        )
        .bind(now)
        .bind(&conv)
        .execute(&mem.pool)
        .await
        .unwrap();
        assert!(mem.forget_conversation(&conv).await.unwrap());
        let row: (i64, Option<String>) = sqlx::query_as(
            "SELECT rev, conversation_id FROM solution_revisions WHERE solution_id = 'sol-1'",
        )
        .fetch_one(&mem.pool)
        .await
        .unwrap();
        assert_eq!(row.0, 1, "revision must still exist");
        assert_eq!(row.1, None, "back-pointer should be NULL");
        let turns: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversation_turns")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(turns.0, 0, "CASCADE drops turns");
    }

    /// `prune_conversations` honors both age and keep-newest, and a dry run
    /// reports the count without deleting.
    #[tokio::test]
    async fn prune_by_age_and_keep_newest() {
        let mem = fresh_memory().await;
        let now = now_secs() as i64;
        let day = 86_400i64;
        // Three old (15 days back), three recent.
        for (i, age) in [15, 15, 15, 1, 1, 1].iter().enumerate() {
            sqlx::query(
                "INSERT INTO conversations (id, started_at, last_seen_at, turn_count) \
                 VALUES (?, ?, ?, 0)",
            )
            .bind(format!("conv-{i}"))
            .bind(now - age * day)
            .bind(now - age * day)
            .execute(&mem.pool)
            .await
            .unwrap();
        }
        // dry_run reports without mutating.
        let n_dry = mem.prune_conversations(7, 0, true).await.unwrap();
        assert_eq!(n_dry, 3, "three conversations are older than 7 days");
        let still: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(still.0, 6, "dry run mustn't delete");
        // Live age-based prune.
        let n = mem.prune_conversations(7, 0, false).await.unwrap();
        assert_eq!(n, 3);
        // Now keep only the newest 2 of the remaining 3.
        let n2 = mem.prune_conversations(0, 2, false).await.unwrap();
        assert_eq!(n2, 1);
        let final_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversations")
            .fetch_one(&mem.pool)
            .await
            .unwrap();
        assert_eq!(final_count.0, 2);
    }

    /// `solution_revisions.conversation_id` is populated when a revision is
    /// written while an active conversation exists — which makes
    /// `solution_conversations` and the conversation→solutions back-reference
    /// work.
    #[tokio::test]
    async fn revision_carries_active_conversation_id() {
        let mem = fresh_memory().await;
        let conv = mem.current_conversation_id().await.unwrap();
        let now = now_secs() as i64;
        sqlx::query(
            "INSERT INTO solutions (id, problem, canon_key, created_at, updated_at) \
             VALUES ('sol-1', 'p', 'p', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO solution_revisions \
             (solution_id, rev, ts, summary, content, notes, conversation_id) \
             VALUES ('sol-1', 1, ?, 's', 'c', '', ?)",
        )
        .bind(now)
        .bind(&conv)
        .execute(&mem.pool)
        .await
        .unwrap();
        let row: (Option<String>,) = sqlx::query_as(
            "SELECT conversation_id FROM solution_revisions WHERE solution_id = 'sol-1'",
        )
        .fetch_one(&mem.pool)
        .await
        .unwrap();
        assert_eq!(row.0.as_deref(), Some(conv.as_str()));
    }

    /// Cosine similarity returns 1.0 for identical vectors, 0.0 for orthogonal,
    /// and is symmetric. Also handles the L2-norm path correctly for
    /// nomic-style L2-normalized embeddings.
    #[test]
    fn cosine_basic_properties() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-6);
        assert!(cosine(&a, &c).abs() < 1e-6);
        assert!((cosine(&a, &b) - cosine(&b, &a)).abs() < 1e-6);
        // Different-length vectors return 0.0 (defense against dim mismatch).
        let d = vec![1.0, 0.0];
        assert_eq!(cosine(&a, &d), 0.0);
        // Zero vector returns 0.0 (no divide-by-zero).
        let z = vec![0.0, 0.0, 0.0];
        assert_eq!(cosine(&a, &z), 0.0);
    }

    /// BLOB encoding is reversible and self-describing — the dim prefix lets
    /// us detect mismatched-model rows at read time.
    #[test]
    fn embedding_blob_round_trip() {
        let v = vec![0.1_f32, -0.2, 0.3, 0.42, -0.5];
        let blob = embedding_to_blob(&v);
        assert_eq!(blob.len(), 4 + 5 * 4);
        let back = blob_to_embedding(&blob).unwrap();
        assert_eq!(v.len(), back.len());
        for i in 0..v.len() {
            assert!((v[i] - back[i]).abs() < 1e-7);
        }
        // Truncated blob → None, not panic.
        assert!(blob_to_embedding(&blob[..blob.len() - 2]).is_none());
        // Too-short for the prefix → None.
        assert!(blob_to_embedding(&[0]).is_none());
    }

    /// `was_semantic_only` is true exactly when token scoring fell below the
    /// recall threshold but the semantic path cleared it — the signal the
    /// dispatch wrapper uses to decide whether to auto-attach the query as a
    /// phrasing.
    #[test]
    fn was_semantic_only_classification() {
        let mut h = RecallHit {
            id: "sol-x".into(),
            problem: "p".into(),
            score: 0.0,
            token_score: 0.0,
            semantic_score: 0.0,
            summary: String::new(),
            links: vec![],
            superseded_by_head: None,
            auto_attached_as_phrasing: false,
        };
        // Token cleared the bar on its own → NOT semantic-only.
        h.token_score = 50.0;
        h.semantic_score = 0.0;
        assert!(!h.was_semantic_only(30.0));
        // Both cleared → NOT semantic-only (token did the work).
        h.token_score = 50.0;
        h.semantic_score = 70.0;
        assert!(!h.was_semantic_only(30.0));
        // Only semantic cleared → IS semantic-only.
        h.token_score = 20.0;
        h.semantic_score = 60.0;
        assert!(h.was_semantic_only(30.0));
        // Neither cleared (this hit shouldn't have been returned at all,
        // but the predicate still has to behave) → NOT semantic-only.
        h.token_score = 0.0;
        h.semantic_score = 0.0;
        assert!(!h.was_semantic_only(30.0));
    }

    /// `auto_attach_phrasing` is idempotent: attaching the same phrasing
    /// twice inserts on the first call and silently no-ops on the second
    /// (FNV-1a hash dedup in the table PK).
    #[tokio::test]
    async fn auto_attach_phrasing_is_idempotent() {
        let mem = fresh_memory().await;
        let now = now_secs() as i64;
        sqlx::query(
            "INSERT INTO solutions (id, problem, canon_key, created_at, updated_at) \
             VALUES ('sol-q', 'p', 'p', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        let http = reqwest::Client::new();
        let q = "How far is Microsoft headquarters from downtown Seattle";
        assert!(mem.auto_attach_phrasing(&http, "sol-q", q).await);
        assert!(!mem.auto_attach_phrasing(&http, "sol-q", q).await);
        let n: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM solution_phrasings WHERE solution_id = 'sol-q'")
                .fetch_one(&mem.pool)
                .await
                .unwrap();
        assert_eq!(n.0, 1);
    }

    /// An attached phrasing makes auto_recall fire on queries that share NO
    /// tokens with the solution's own problem text. This is the "we'll never
    /// surface this if asked differently" gap closed.
    #[tokio::test]
    async fn alias_phrasings_extend_recall_to_unrelated_wording() {
        let mem = fresh_memory().await;
        let now = now_secs() as i64;
        // Solution's own problem mentions Redmond / Seattle — query about
        // "Microsoft headquarters" shares ZERO tokens with this.
        sqlx::query(
            "INSERT INTO solutions (id, problem, canon_key, concept_key, created_at, updated_at) \
             VALUES ('sol-z', 'Redmond is 40 miles east of Seattle - validate', \
             'redmond 40 miles east seattle validate', \
             'redmond 40 miles east seattle validate', ?, ?)",
        )
        .bind(now)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO solution_revisions \
             (solution_id, rev, ts, summary, content, notes) \
             VALUES ('sol-z', 1, ?, 's', 'c', '')",
        )
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        let http = reqwest::Client::new();
        // Baseline: a query about Microsoft HQ shares no tokens / tags. No hit.
        let baseline = mem
            .auto_recall(
                &http,
                "drive time from downtown to Microsoft headquarters",
                5,
            )
            .await;
        assert!(
            baseline.is_empty(),
            "without phrasings, the unrelated-wording query must NOT recall this solution"
        );
        // Attach a phrasing that shares vocabulary with the new query.
        let phrasing = "How far is Microsoft headquarters from downtown Seattle?";
        let canon = crate::provider::canonical_query(phrasing);
        let concept = concept_key_of(phrasing);
        let hash = phrasing_hash(&canon);
        sqlx::query(
            "INSERT INTO solution_phrasings \
             (solution_id, hash, phrasing, canon_key, concept_key, created_at) \
             VALUES ('sol-z', ?, ?, ?, ?, ?)",
        )
        .bind(&hash)
        .bind(phrasing)
        .bind(&canon)
        .bind(&concept)
        .bind(now)
        .execute(&mem.pool)
        .await
        .unwrap();
        // With the phrasing attached, the query now lands on sol-z via the
        // phrasing's concept tokens (microsoft, headquarters, downtown,
        // seattle) overlapping the new query's tokens.
        let after = mem
            .auto_recall(
                &http,
                "drive time from downtown to Microsoft headquarters",
                5,
            )
            .await;
        assert!(
            !after.is_empty(),
            "with a phrasing attached, the alternate wording must now recall sol-z"
        );
        assert_eq!(after[0].id, "sol-z");
    }
}
