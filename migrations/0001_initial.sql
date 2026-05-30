-- Migration 0001: initial schema for memory + solutions + synonyms.
--
-- The memory module wraps three persistent tool families into one indexed
-- SQLite database. Foreign-key CASCADE removes a solution's revisions / tags /
-- outbound links automatically; dangling INCOMING links from other solutions
-- are cleaned by the application in the same transaction as the delete.
--
-- All `CREATE TABLE` statements are idempotent (`IF NOT EXISTS`) so re-running
-- the migration over an existing-but-version-tracked database is safe.

PRAGMA foreign_keys = ON;

-- Key-value memos the model writes between sessions. `tags_json` is a JSON
-- array (small per-entry, no need for an index — substring search hits LOWER()).
CREATE TABLE IF NOT EXISTS memory (
    scope       TEXT    NOT NULL DEFAULT '',
    key         TEXT    NOT NULL,
    value       TEXT    NOT NULL,
    tags_json   TEXT    NOT NULL DEFAULT '[]',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (scope, key)
);
CREATE INDEX IF NOT EXISTS memory_updated ON memory(updated_at DESC);

-- Proposed solutions. `canon_key` and `concept_key` are pre-computed at insert
-- time via crate::provider::canonical_query / concept_tokens, so recall is
-- a sub-millisecond indexed lookup regardless of how many solutions exist.
CREATE TABLE IF NOT EXISTS solutions (
    id          TEXT PRIMARY KEY,
    problem     TEXT NOT NULL,
    canon_key   TEXT NOT NULL,
    concept_key TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS solutions_canon   ON solutions(canon_key);
CREATE INDEX IF NOT EXISTS solutions_concept ON solutions(concept_key);
CREATE INDEX IF NOT EXISTS solutions_updated ON solutions(updated_at DESC);

-- Revision history. CASCADE on the FK removes a solution's revisions in lockstep.
CREATE TABLE IF NOT EXISTS solution_revisions (
    solution_id TEXT    NOT NULL,
    rev         INTEGER NOT NULL,
    ts          INTEGER NOT NULL,
    summary     TEXT    NOT NULL,
    content     TEXT    NOT NULL,
    notes       TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (solution_id, rev),
    FOREIGN KEY (solution_id) REFERENCES solutions(id) ON DELETE CASCADE
);

-- Tag index. `tag` is the lowercased form (used by the index); `label` keeps
-- the original casing for display.
CREATE TABLE IF NOT EXISTS solution_tags (
    solution_id TEXT NOT NULL,
    tag         TEXT NOT NULL,
    label       TEXT NOT NULL,
    PRIMARY KEY (solution_id, tag),
    FOREIGN KEY (solution_id) REFERENCES solutions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS solution_tags_tag ON solution_tags(tag);

-- Typed graph edges. Auto-reciprocal pairs (`supersedes` ↔ `superseded-by`,
-- `depends-on` ↔ `dependency-of`) are inserted as TWO rows by the application
-- in a single transaction. Free-form `kind` is allowed; unknown kinds are
-- treated as symmetric. The `solution_links_to` index lets `solution_forget`
-- clean dangling INCOMING edges in O(log N).
CREATE TABLE IF NOT EXISTS solution_links (
    from_id TEXT NOT NULL,
    kind    TEXT NOT NULL,
    to_id   TEXT NOT NULL,
    note    TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (from_id, kind, to_id),
    FOREIGN KEY (from_id) REFERENCES solutions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS solution_links_to ON solution_links(to_id);

-- Learned single-token aliases. `synonym_add` upserts; the loader at startup
-- copies these into an in-process Arc<RwLock<HashMap>> that
-- crate::provider::canonical_query reads on every token.
CREATE TABLE IF NOT EXISTS synonyms (
    token      TEXT PRIMARY KEY,
    canonical  TEXT NOT NULL,
    note       TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL
);
