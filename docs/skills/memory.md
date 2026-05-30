# Memory & solutions — `memory_*` / `solution_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/memory.rs`](../../src/skills/memory.rs) |
| **Tools** | `memory_save`, `memory_get`, `memory_list`, `memory_search`, `memory_forget`, `solution_record`, `solution_find`, `solution_show`, `solution_list`, `solution_update`, `solution_forget`, `solution_link`, `solution_unlink`, `solution_graph`, `solution_related` |
| **Network** | none (purely local on-disk store) |
| **Default** | **on** (`[memory].enabled`) |
| **Config** | [`config/18-memory.toml`](../../config/18-memory.toml) |

## What it does
Two related families share one on-disk JSONL store (`[memory].dir`, default
`.lodestone-memory/`):

- **`memory_*`** — a simple key→value store the model can write to remember
  anything across sessions and restarts. Optional `scope` namespaces a group
  (e.g. `"user-prefs"`) and optional `tags` make it discoverable via
  `memory_search`.
- **`solution_*`** — proposed solutions to past problems, with **full revision
  history**. `solution_record` saves one. Later, `solution_find` surfaces
  matching prior solutions as **SUGGESTIONS** — *advisory, not prescriptive* —
  so the model can build on (or revise) what worked before without treating the
  old answer as authoritative. `solution_update` appends a new revision; prior
  revisions stay queryable via `solution_show`.

Entries are **local only** — they are never advertised in the constellation
digest and never cross the network.

## Storage & durability
- Two append-only JSONL journals: `memory.jsonl` and `solutions.jsonl`. Every
  save/update/forget appends one line.
- On startup the server **replays** the journals into memory and **atomically
  rewrites** each file with the current snapshot (no tombstones, no superseded
  lines) — so the files stay bounded in size.
- A `deleted: true` line is a tombstone; replay drops the corresponding entry.

## Similarity recall (`solution_find`)
Ranked from strongest to weakest:

1. **Exact canonical** — same word order, same stop-worded, **synonym-folded**
   content (`canonical_query`). Score 100.
2. **Exact concept** — same content tokens, ignoring order (`concept_tokens`).
   Score 80.
3. **Fuzzy concept overlap** — Jaccard over stemmed token sets. Score
   `20 + 40·jaccard`, so a partial-but-strong overlap can still surface.
4. **Substring** — case-insensitive substring of the problem text. Score 15.
5. **Tag boost** — `+5` per tag the candidate shares with the query's `tags`
   filter. A tag-only match (no text signal at all) surfaces at score 10.

Solutions matching by multiple paths take the strongest single path's score, plus
the tag boost. `solution_find` returns the top `max` (default 5, cap 20), with
the match path and score shown for each. The output is **explicitly labeled
"SUGGESTED prior solutions (advisory — may be stale; verify before reusing)"** —
the model is expected to look it over, not act on it blindly.

### Synonym fold
Before tokens are committed to the index or matched against it, a small
**single-token alias table** (in `src/provider.rs`) collapses common
equivalents — so e.g. `k8s`/`kubernetes`, `ssl`/`tls`, `gh`/`github`,
`js`/`javascript`, `setup`/`config`/`conf`/`configure` all hash to the same
canonical key. This change affects the search cache too, so a reworded query
also reuses cached search results. Add new aliases by editing `fold_synonym` in
`src/provider.rs`.

## Relation graph (`solution_link` / `solution_unlink` / `solution_graph` / `solution_related`)
Beyond lexical similarity, solutions can be connected by **typed, auto-reciprocal
edges** so the model can navigate by relation rather than by wording.

- `solution_link { from, kind, to, note? }` declares an edge. Known directional
  pairs flip on the target: `supersedes` ↔ `superseded-by`,
  `depends-on` ↔ `dependency-of`. Anything else (`alternative-to`,
  `related-to`, `see-also`, or any custom kind) is symmetric — the same kind is
  added to the target.
- `solution_unlink { from, kind, to }` removes the edge from both ends.
- `solution_graph { id, depth? }` (default 2 hops, max 5) renders the
  **explicit-link** subgraph around one solution: typed arrows from each node
  to the next, with problem snippets.
- `solution_related { id, max? }` returns a **combined** ranked list: explicit
  links (weight 30 per link), shared tags (weight 2 each), concept-token
  Jaccard (weight `20·overlap`). The output shows which signals contributed.
- `solution_forget` automatically strips dangling **incoming** edges from other
  solutions, so the graph stays consistent after a delete.
- `solution_show` includes a `Links:` section listing every outgoing edge.

Use `solution_link` whenever a new solution either replaces an older one
(`supersedes`), builds on a foundation (`depends-on`), is an alternative
(`alternative-to`), or is simply worth knowing about together (`related-to` /
`see-also`).

## Tools

### `memory_*`
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `memory_save` | `key`, `value`, `scope?`, `tags?` | Save/upsert a memory. |
| `memory_get` | `key`, `scope?` | Exact lookup by key (+scope). |
| `memory_list` | `scope?`, `prefix?`, `max?` | List keys with previews. |
| `memory_search` | `query`, `scope?`, `tag?`, `max?` | Substring search across key/value/tags. |
| `memory_forget` | `key`, `scope?`, `confirm?`, `trust?` | **Destructive** — guarded. |

### `solution_*`
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `solution_record` | `problem`, `summary`, `content`, `notes?`, `tags?` | Save a proposed solution; returns its id (`sol-N`). |
| `solution_find` | `query?`, `tags?`, `max?` | Surface SUGGESTED prior solutions (advisory). Needs `query` or `tags`. |
| `solution_show` | `id` | Full history (all revisions, oldest→newest). |
| `solution_list` | `max?` | List by last-updated, newest first. |
| `solution_update` | `id`, `summary`, `content`, `notes?`, `tags?` | Append a new revision; prior revisions are kept. Pass `tags` to replace, `[]` to clear. |
| `solution_forget` | `id`, `confirm?`, `trust?` | **Destructive** — guarded. Drops all revisions. Strips dangling incoming links from other solutions. |
| `solution_link` | `from`, `kind`, `to`, `note?` | Declare a typed relation between two solutions; reciprocal is auto-added (e.g. `supersedes`→`superseded-by`). |
| `solution_unlink` | `from`, `kind`, `to` | Remove a typed relation; the reciprocal on the target is also removed. |
| `solution_graph` | `id`, `depth?` | BFS over **explicit** links around one solution (default 2 hops, max 5). |
| `solution_related` | `id`, `max?` | Ranked combination: explicit links (weight 30/link) + shared tags (2/tag) + concept-token Jaccard (20·overlap). |

## Configuration & gating
| Key | Default | Notes |
| --- | --- | --- |
| `[memory].enabled` | `false` | Master switch. |
| `[memory].dir` | `.lodestone-memory` | Where the two JSONL files live. |
| `[memory].allow_destructive` | `false` | Pre-authorize `memory_forget` / `solution_forget`. |
| `[memory].max_entries` | `10000` | Soft cap per store. |
| `[memory].max_value_chars` | `64000` | Per-value cap. |

Env overrides: `LODESTONE_MEMORY_ENABLED`, `LODESTONE_MEMORY_DIR`,
`LODESTONE_MEMORY_ALLOW_DESTRUCTIVE`.

## Example uses
- **Remember a user preference** — `memory_save { key: "prefer-rg-over-grep",
  value: "yes", scope: "user-prefs", tags: ["preferences"] }`.
- **Recall it later** — `memory_get { key: "prefer-rg-over-grep", scope:
  "user-prefs" }`.
- **Record what worked** — `solution_record { problem: "Deploy lodestone behind
  nginx with TLS", summary: "Reverse proxy + ACME", content: "…", tags:
  ["deployment", "nginx"] }`.
- **Recall a similar past attempt** — `solution_find { query: "lodestone TLS
  routing", tags: ["deployment"] }` — surfaces any prior entry whose problem
  matches by canonical, concept, fuzzy overlap, substring, or shared tag.
- **Revise as you learn more** — `solution_update { id: "sol-3", summary:
  "Reverse proxy + ACME (now with HSTS preload)", content: "…", tags:
  ["deployment", "nginx", "hsts"] }` — adds revision 2; rev 1 stays in history.

## See also
- [tools.md](../tools.md) — full tool reference.
- [golden-rules.md](../golden-rules.md) — rule 8 (destructive actions guarded),
  rule 9 (one tool per method — `memory_*` and `solution_*` are split this way
  rather than one overloaded "remember" tool).
