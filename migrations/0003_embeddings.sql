-- Migration 0003: semantic recall + per-solution phrasings.
--
-- Closes the "we only solved this once, will we surface it again when asked
-- differently?" gap. Two changes:
--
--   1. `solutions.embedding` — optional vector of the (problem + summary)
--      text. Computed at record / update time via the OpenAI-compatible
--      embeddings endpoint pointed at by [memory].embedding_endpoint (the
--      same :1234 that LM Studio serves). NULL when embeddings are disabled
--      or the endpoint was unreachable at write time. Stored as a little-
--      endian byte-packed sequence: `[u32 dim][f32 * dim]`.
--
--   2. `solution_phrasings` — a many-to-one table letting one solution
--      accumulate multiple ways the same underlying question has been
--      asked. Each row carries its own canonical / concept keys (for token
--      scoring) and its own embedding (for semantic scoring). Recall scores
--      against the union of the solution's own problem AND every phrasing,
--      taking the best match.
--
-- Together these mean the recall layer's hit rate grows over time as the
-- model encounters and attaches new phrasings, instead of degrading to "only
-- the original phrasing matches."

PRAGMA foreign_keys = ON;

ALTER TABLE solutions ADD COLUMN embedding BLOB;

CREATE TABLE IF NOT EXISTS solution_phrasings (
    solution_id TEXT    NOT NULL,
    -- A short stable hash of the phrasing text so the same phrasing can't be
    -- added twice. We compute this in application code (FNV-1a over the
    -- canonical key) to avoid a separate sqlite function dependency.
    hash        TEXT    NOT NULL,
    phrasing    TEXT    NOT NULL,
    canon_key   TEXT    NOT NULL,
    concept_key TEXT,
    embedding   BLOB,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (solution_id, hash),
    FOREIGN KEY (solution_id) REFERENCES solutions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS solution_phrasings_canon   ON solution_phrasings(canon_key);
CREATE INDEX IF NOT EXISTS solution_phrasings_concept ON solution_phrasings(concept_key);
