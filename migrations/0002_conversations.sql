-- Migration 0002: conversation tracking.
--
-- The dispatch wrapper writes one row to `conversation_turns` per tool call so
-- the model can answer two questions about a remembered solution:
--   * "what else happened in this conversation?" — list its turns
--   * "what conversation was this a part of?"   — look at the linked rev row
--
-- Session identity is an idle-gap heuristic (no client cooperation required):
-- after a configured period of silence, the next call starts a fresh
-- conversation id. The current id is materialized in `conversations` on its
-- first turn so consumers can name it without a join.
--
-- A `conversation_id` column is added to `solution_revisions` so each revision
-- knows the conversation it came out of. NULL is allowed (older revisions
-- predate this migration; revisions written when memory is disabled also have
-- no conversation).

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS conversations (
    id           TEXT    PRIMARY KEY,
    started_at   INTEGER NOT NULL,
    last_seen_at INTEGER NOT NULL,
    turn_count   INTEGER NOT NULL DEFAULT 0,
    -- The first non-empty query seen in this conversation, kept verbatim for
    -- listing/preview. NULL when the conversation only saw query-less tools.
    first_query  TEXT
);
CREATE INDEX IF NOT EXISTS conversations_last_seen ON conversations(last_seen_at DESC);

-- One row per tool call. `seq` is a per-conversation monotonic counter so we
-- can return turns in their true order even when two calls share a timestamp.
-- `query` is NULL for tools that don't take a free-text query; `response_excerpt`
-- is a short capped slice of the tool's response (cap enforced in code).
CREATE TABLE IF NOT EXISTS conversation_turns (
    conversation_id  TEXT    NOT NULL,
    seq              INTEGER NOT NULL,
    ts               INTEGER NOT NULL,
    tool_name        TEXT    NOT NULL,
    query            TEXT,
    response_excerpt TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (conversation_id, seq),
    FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS conversation_turns_ts ON conversation_turns(ts DESC);

-- Each revision now records the conversation it was written from. NULL means
-- "no conversation context" (legacy rows; or memory was disabled at write).
ALTER TABLE solution_revisions ADD COLUMN conversation_id TEXT;
CREATE INDEX IF NOT EXISTS solution_revisions_conv ON solution_revisions(conversation_id);
