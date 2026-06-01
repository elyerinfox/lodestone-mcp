# Persistent memory

Lodestone has a SQLite-backed memory layer that survives across sessions. It
is **on by default** — to silence it, set `[memory].enabled = false` in
`config/18-memory.toml`. When disabled, none of the tools below exist and the
intrinsic recall wrapper is inert; the rest of the server is unaffected.

The layer has three discrete responsibilities, deliberately separated so the
model can reach for the right shape of memory without overloading one tool:

1. **Key→value notes** — small free-form remembered facts (`memory_*`).
2. **Solutions** — recorded "how I solved this" entries with revisions, tags,
   and a typed graph of relationships (`solution_*`).
3. **Synonyms** — single-token aliases that get folded everywhere queries are
   normalized, so a rewording finds the prior entry (`synonym_*`).

On top of those, a **frictionless on-ramp** lets the model not-think about
which shape to use:

- **`remember { text, as? }`** — auto-derives a key, auto-extracts tags,
  writes a memo by default. Text shaped like a recipe (`→`, starts with
  `to`/`when`/`if`/`fix:`/`solution:`/`use`) auto-classifies as a solution.
  Force the shape with `as: "fact" | "solution"`.
- **`remember_fact { text }`** — always a memo, no classifier.
- **`remember_solution { text, problem?, summary? }`** — always a solution.
  First sentence becomes the problem; rest becomes the content.
- **`recall { query, kinds?, limit? }`** — one merged hit list across memos
  + solutions + phrasings. Replaces calling `memory_search` and
  `solution_find` separately.

And another piece sits *over* all of these: **intrinsic recall**, a dispatch
wrapper that auto-prepends a "💡 prior solutions" + "📝 N facts you noted"
preamble to every query-bearing tool call. The model never has to call
`solution_find` or `memory_search` — recall fires by itself.

> All recall hits are **advisory**. Old solutions may be stale; the model is
> instructed to verify before reusing, and to record an update when it learns
> better. The whole point of recording a solution is so the *next* attempt
> starts from real evidence, not from scratch — but advisory is not
> authoritative.

## Architecture at a glance

```mermaid
flowchart LR
    Query["query / arguments"]
    Canon["canonical_query<br/>(lowercase + stopword strip<br/>+ synonym fold)"]
    Concept["concept_tokens<br/>(set form, order-free)"]
    Score["score_solution_row<br/>exact &gt; concept &gt; fuzzy &gt; substring<br/>+ tag boost"]
    Walk["walk_supersession_head<br/>(bounded, cycle-safe)"]
    Preamble["💡 N prior solutions<br/>(advisory)"]
    Tool["tool's normal response"]

    Synonyms[("synonyms table<br/>(RwLock map)")]
    Solutions[("solutions + revisions<br/>+ tags + links")]

    Query --> Canon --> Score
    Query --> Concept --> Score
    Synonyms -. fold .-> Canon
    Synonyms -. fold .-> Concept
    Solutions --> Score
    Score --> Walk
    Walk --> Preamble
    Solutions -. outgoing links .-> Preamble
    Preamble --> Tool
```

## Intrinsic recall — the dispatch wrapper

Every tool whose arguments carry a free-text `query` (the entire search family,
arxiv / wikipedia / pubmed / openalex / hf_* / standards / rfc / news / osm_* /
task_run / …) is wrapped at dispatch time. When the wrapper sees a non-empty
`query`, it:

1. Canonicalizes the query (folding synonyms).
2. Scores all stored solutions and keeps the top hits above a quality
   threshold.
3. For each kept hit, walks the `superseded-by` chain forward to find the
   current head.
4. Pulls each hit's outgoing typed links.
5. Runs a LIKE search against the memo store for the same query (capped at
   3 hits) when `[memory].auto_recall_facts` is on.
6. Prepends two preamble blocks to the tool's response: "💡 N prior
   solutions" from the solution store and "📝 N facts you noted" from the
   memo store. Either half can be empty; the wrapper skips the block in
   that case.

```mermaid
sequenceDiagram
    participant Model
    participant Wrapper as Dispatch wrapper
    participant Memory
    participant Tool

    Model->>Wrapper: call("web_search", {query: "deploy lodestone behind nginx"})
    Wrapper->>Memory: enabled? + auto_recall(query, 3)
    Memory-->>Wrapper: hits[], each with links + supersession head
    Wrapper->>Tool: run skill normally
    Tool-->>Wrapper: result.content = [search results]
    Wrapper->>Wrapper: prepend "💡 N prior solutions ..." preamble
    Wrapper-->>Model: result.content = [preamble, search results]
```

Tools in the **memory / solution / synonym / conversation / remember /
recall** family are excluded from the wrapper — otherwise calling
`solution_find` would surface its own results as a recall preamble, recurse,
and become noise.

### What a preamble looks like

```
💡 2 prior solutions matching this (advisory — verify before reusing):
  • sol-3 (score 78.0): Deploy lodestone behind nginx with TLS
    ⚠ superseded — current head is sol-9; prefer it unless you specifically need the older approach
    summary: Old approach using certbot directly on the host
    links: ─superseded-by→ sol-5
    ↳ solution_graph id="sol-3" to walk further, solution_related id="sol-3" for ranked neighbors
  • sol-12 (score 64.5): Reverse-proxy lodestone with Caddy auto-TLS
    summary: Single-binary alternative; ACME without certbot
    links: ─related-to→ sol-9  ─depends-on→ sol-7
    ↳ solution_graph id="sol-12" to walk further, solution_related id="sol-12" for ranked neighbors

📝 1 fact you noted about this (advisory):
  • lodestone-tls-cert-path: certs live under /etc/lodestone/tls/ — Caddy needs read access via the lodestone group
───
```

The `⚠ superseded` line is load-bearing: surfacing an obsolete hit without
pointing at the chain head would quietly steer the model into stale prior work,
which is the opposite of what the memory layer is for.

The `📝` half is gated by `[memory].auto_recall_facts` (default `true`) AND
the master `[memory].auto_recall`. Turn the fact half off independently when
the memo store is noisier than the solution store for a given session.

## Key→value notes — `memory_*`

A flat, scoped key→value store for anything that doesn't fit the
problem/solution shape — environment notes, "the staging cluster is in
`gke_a`", a one-off reminder.

| Tool | Purpose |
| --- | --- |
| `memory_save` | Write/overwrite `value` at `key`. Optional `scope`, `tags`, `note`. |
| `memory_get` | Read one key. |
| `memory_list` | List keys, optionally filtered by `scope` / `tags` prefix. |
| `memory_search` | Full-text-ish search across keys, values, notes. |
| `memory_forget` | Delete a key. Destructive — goes through the confirm-token guard. |

Notes are **not** considered by intrinsic recall. They're a separate surface;
use solutions when you want recall.

## Solutions — `solution_*`

A solution is a recorded answer to a recorded problem, with full revision
history. Use it when an approach is worth remembering — debugging recipes,
deployment patterns, "how I solved the X bug last quarter."

| Tool | Purpose |
| --- | --- |
| `solution_record` | Create a new solution for a `problem`. Stores rev 1 immediately. Optional `tags`. |
| `solution_find` | Explicit recall — same scoring as intrinsic recall but on demand. |
| `solution_show` | Full detail: every revision, tags, outgoing/incoming links. |
| `solution_list` | List by tag / canon prefix / recency. |
| `solution_update` | Append a new revision to an existing solution. History is preserved. |
| `solution_forget` | Delete a solution (cascades to revisions / tags / links). Destructive. |

### Ranking

`score_solution_row` is a deterministic ranker, not an embedding model. The
scoring goes:

| Match | Score |
| --- | --- |
| Exact canonical-query match | 100 |
| Exact concept-token match (order-free) | 80 |
| Fuzzy concept-token Jaccard | 20 + 40·j |
| Substring in problem / summary | 15 |
| Per shared tag | +5 |

Threshold for intrinsic recall is 30, so a one-tag-overlap-only match doesn't
fire — the preamble stays high signal.

### The graph layer

Solutions can be linked with **typed, auto-reciprocal edges**. Writing
`supersedes` from A→B automatically writes `superseded-by` from B→A; the same
holds for `depends-on` ↔ `dependency-of`. Symmetric kinds (`alternative-to`,
`related-to`, `see-also`, or any custom kind) write the same kind in the other
direction.

```mermaid
flowchart LR
    sol9["sol-9<br/>Caddy reverse proxy<br/>(current)"]
    sol5["sol-5<br/>nginx + certbot via apt"]
    sol3["sol-3<br/>nginx + certbot host install"]
    sol7["sol-7<br/>DNS / A record at registrar"]
    sol12["sol-12<br/>Caddy auto-TLS"]
    sol14["sol-14<br/>fail2ban for the public port"]

    sol9 -- supersedes --> sol5
    sol5 -. superseded-by .-> sol9
    sol5 -- supersedes --> sol3
    sol3 -. superseded-by .-> sol5
    sol9 -- depends-on --> sol7
    sol9 -- related-to --- sol12
    sol9 -- see-also --- sol14
```

| Tool | Purpose |
| --- | --- |
| `solution_link` | Create a typed edge `from → to`. Reciprocal written automatically. |
| `solution_unlink` | Remove an edge (and its reciprocal). |
| `solution_graph { id, depth?=2 }` | BFS-walk the typed edges from `id` out to `depth` (capped at 5). Returns nodes + edges. |
| `solution_related { id, max?=10 }` | Ranked neighbor list combining explicit-link weight (30/link) + shared tags (2/tag) + concept-token Jaccard (20·overlap). |

### Supersession chain walking

When intrinsic recall surfaces a hit, it walks `superseded-by` edges forward
from the hit until it lands on a node nothing has superseded — the **head**.
The walk is bounded (5 hops) and uses a visited set so a malformed cycle in
`solution_links` can never lock the recall path.

```mermaid
flowchart LR
    A["sol-a<br/>(matched by query)"] -- superseded-by --> B[sol-b]
    B -- superseded-by --> C[sol-c]
    C -- superseded-by --> D["sol-d<br/>★ head"]
    D -. nothing supersedes .-> done((stop))
```

The preamble then reads:

```
⚠ superseded — current head is sol-d; prefer it unless you specifically need the older approach
```

Only directional kinds (`supersedes` / `superseded-by`) get walked
transitively. Symmetric kinds stay at one hop in the preamble because
"related-to-related-to" is just weaker relatedness — chasing it dilutes the
signal.

## Conversations — `conversation_*` / `solution_conversations`

The dispatch wrapper writes one row to `conversation_turns` per tool call.
Turns are grouped into `conversations` by an **idle-gap heuristic** — 30
minutes of silence ends one conversation and starts the next. No client
cooperation required; no session id is asked for. When `solution_record` or
`solution_update` runs inside a conversation, the new revision is back-linked
to it.

A conversation is **has-many** to turns (`conversation_turns.conversation_id`)
and **has-many** to solution revisions
(`solution_revisions.conversation_id`). Because a single solution can be
updated across multiple conversations (revisions 1, 2, 3 each from a different
session), a solution is **many-to-many** to conversations via revisions.

```mermaid
erDiagram
    conversations ||--o{ conversation_turns : "has many"
    conversations ||--o{ solution_revisions : "produces"
    solutions     ||--o{ solution_revisions : "has many"
    conversations {
        TEXT    id PK
        INTEGER started_at
        INTEGER last_seen_at
        INTEGER turn_count
        TEXT    first_query
    }
    conversation_turns {
        TEXT    conversation_id FK
        INTEGER seq PK
        INTEGER ts
        TEXT    tool_name
        TEXT    query
        TEXT    response_excerpt
    }
    solution_revisions {
        TEXT    solution_id FK
        INTEGER rev PK
        INTEGER ts
        TEXT    summary
        TEXT    content
        TEXT    conversation_id FK
    }
```

The dispatch wrapper looks like this end-to-end:

```mermaid
sequenceDiagram
    participant Model
    participant Wrapper as Dispatch wrapper
    participant Memory
    participant Tool

    Model->>Wrapper: call(tool, args)
    Wrapper->>Memory: current_conversation_id()
    Memory-->>Wrapper: conv-1717... (reused or freshly minted)
    opt args carry a query
        Wrapper->>Memory: auto_recall(query, 3)
        Memory-->>Wrapper: hits[]
        Wrapper->>Wrapper: prepend "💡 prior solutions" preamble
    end
    Wrapper->>Tool: run skill
    Tool-->>Wrapper: result
    Wrapper->>Memory: record_turn(conv_id, tool, query, excerpt)
    Wrapper-->>Model: result (with preamble if any)
```

| Tool | Purpose |
| --- | --- |
| `conversation_list { max?=20 }` | Recent conversations, most-recently-active first. |
| `conversation_show { id, max?=100 }` | Walk one conversation chronologically — every tool call, with query and a short response excerpt, plus the solutions whose revisions were produced. |
| `solution_conversations { id }` | The conversation(s) a solution came from, grouped by which revisions each one produced. |

`solution_show` also prints the `conversation_id` next to each revision. So
the typical traversal flow is:

```
[recall preamble surfaces sol-3]
  → solution_show id="sol-3"     (see its revisions and which conversation each came from)
  → conversation_show id="conv-1717..."
     (see "what else happened in that conversation": adjacent searches, fetches, the surrounding context)
```

### Cleanup

- **`conversation_forget { id, confirm?, trust? }`** — delete one
  conversation. CASCADE drops its `conversation_turns`; the
  `solution_revisions.conversation_id` back-pointer is set to NULL on every
  revision that referenced it (revision content is preserved — only the
  back-link to the now-gone conversation is dropped). The active in-process
  conversation tracker is also cleared when the deleted id was the one in
  use, so the next tool call starts a fresh conversation. Destructive,
  goes through the confirm-token guard; `[memory].allow_destructive = true`
  pre-authorizes.
- **`conversation_prune { older_than_days?, keep_newest?, dry_run?, confirm?, trust? }`** —
  bulk delete by retention policy. Falls back to the configured
  `[memory].conversation_retention_days` / `max_conversations` when neither
  argument is set. `dry_run = true` reports the count without deleting (and
  without asking for a confirm token) — use this to validate the policy
  before flipping a live prune.
- **Startup pruning** — when `[memory].prune_on_startup = true`, the
  configured retention rules are applied once at boot. Off by default so a
  misconfigured policy doesn't surprise-delete history on first upgrade.

### Known limitations

- **Identity is per-process, not per-client.** MCP doesn't hand the server a
  stable per-user session id, so the wrapper uses one global "active
  conversation." If one server is shared by multiple concurrent clients,
  their turns will mix. The local-LLM-runner case (one client at a time) is
  the design target.

## Semantic recall — embeddings + phrasings

The big risk of any recall system is that the **first time you record a
solution, you commit to one vocabulary** — and the next time the same
underlying question gets asked with different words, the recall layer
silently fails to surface it. Two complementary mechanisms close that gap:

### Embeddings (semantic match)

When `[memory].embedding_endpoint` points at an OpenAI-compatible
`/v1/embeddings` server (LM Studio serves one at
`http://127.0.0.1:1234/v1/embeddings`), every recorded solution and every
attached phrasing is embedded at write time and the vector is stored as a
length-prefixed BLOB on the row. At recall time, the query is embedded too,
and the scoring path takes **`max(token_score, semantic_score)`** per
solution. Cosine similarity in `[embedding_threshold, 1.0]` is linearly
mapped onto `[40, 100]` — a hit at exactly the threshold is enough to fire
the preamble on its own, near-perfect matches outscore even exact canonical
token matches.

- **Off by default.** Empty `embedding_endpoint` skips the whole semantic
  path; recall continues with token-only scoring. No network dep when off.
- **Graceful degradation.** A down embedding server doesn't error the
  write — the row just lands with `embedding = NULL` and is invisible to
  semantic recall until re-embedded (via `solution_update`).
- **Storage cost.** ~3 KB per vector for nomic-embed (768 dims × 4 bytes +
  prefix). Negligible for typical solution counts.

### Phrasings (`solution_alias_add` / `solution_alias_remove`)

When the model notices the same solution would have applied to a question
asked in a way the original problem text wouldn't have matched, it can
attach the new phrasing:

```json
solution_alias_add {
  "id": "sol-3",
  "phrasing": "How far is Microsoft HQ from downtown Seattle?"
}
```

Each phrasing carries its own `canon_key` / `concept_key` (so token
overlap considers it) and its own embedding (so semantic similarity does
too). Recall scores against the **union** of the solution's own problem
text and every attached phrasing, taking the best match. A solution
accumulates ways it's been asked over time, turning the "one wording
locks in the recall" failure mode into a self-improving loop.

```mermaid
flowchart LR
    Q["query"]
    QE["query embedding"]
    QT["query tokens"]
    S["solution row<br/>problem · concept_key · embedding"]
    P1["phrasing 1<br/>concept_key · embedding"]
    P2["phrasing 2<br/>concept_key · embedding"]
    SC["score_solution_row<br/>(token path)"]
    SEM["cosine similarity<br/>(semantic path)"]
    MAX["max(token, semantic)<br/>per solution"]
    PRE["💡 preamble<br/>if ≥ recall_threshold"]

    Q --> QT --> SC
    Q --> QE --> SEM
    S --> SC
    P1 --> SC
    P2 --> SC
    S --> SEM
    P1 --> SEM
    P2 --> SEM
    SC --> MAX
    SEM --> MAX
    MAX --> PRE
```

| Tool | Purpose |
| --- | --- |
| `solution_alias_add { id, phrasing }` | Attach an alternate phrasing of the same underlying question. Best-effort embedded for semantic recall. |
| `solution_alias_remove { id, phrasing }` | Detach a previously-added phrasing. Match is by canonical form. |

### Auto-aliasing

When the dispatch wrapper sees that the **top recall hit** fired *only* via
the semantic path (its token-overlap score didn't clear `recall_threshold`,
but the embedding cosine did) and the query carries at least
`auto_alias_min_query_tokens` concept tokens, it **automatically attaches
the query as a new phrasing** on that solution. The preamble adds a small
visible note:

```
✎ noted this phrasing on the solution for next time (auto-aliased)
```

Result: future token-shaped recall finds the same solution without
re-running embeddings, and the recall layer's hit rate **grows with use**
rather than ossifying around whatever wording the model happened to use
first. The noise guard (`auto_alias_min_query_tokens`, default 3) stops a
single common noun (e.g. "campus") from attaching itself to whichever
solution it semantically lands on. Set `auto_alias_on_semantic_recall =
false` to disable.

## Synonyms — `synonym_*`

A single-token alias map: `token` → `canonical`. The fold runs in both
`canonical_query` (used for the search cache key and for solution scoring) and
`concept_tokens` (used for fuzzy match). The store ships **empty** — there's
no hardcoded `k8s`→`kubernetes` table; the model and user grow it as they
learn.

| Tool | Purpose |
| --- | --- |
| `synonym_add` | Insert `token` → `canonical` (both lowercased). |
| `synonym_remove` | Delete an entry. |
| `synonym_list` | Read the whole table. |

A practical effect: once `synonym_add token="k8s" canonical="kubernetes"` is
stored, `web_search { query: "k8s deploy" }` and `web_search { query:
"kubernetes deploy" }` hit the same cache slot, *and* recall the same prior
solutions. The same fold is what makes intrinsic recall robust to surface
wording changes.

## Configuration

Everything lives in `config/18-memory.toml`. The layer is intentionally
lever-rich — recall verbosity, conversation rotation, retention, and
per-behavior switches are all separately tunable so operators can dial it in
without writing code.

```toml
[memory]
# --- Family switches ---------------------------------------------------------
enabled            = true             # silence the whole family when false
dir                = ".lodestone-memory"
allow_destructive  = false            # pre-authorize *_forget / conversation_prune
max_entries        = 10000            # soft cap per store
max_value_chars    = 64000            # per-value cap

# --- Intrinsic recall --------------------------------------------------------
auto_recall              = true       # master switch for the dispatch-wrapper preamble
auto_recall_facts        = true       # include the "📝 facts you noted" half of the preamble
recall_threshold         = 30.0       # match score floor; lower = chattier
recall_max_hits          = 3          # cap preamble length
superseded_walk_max_hops = 5          # supersession-head walker; 0 disables warning

# --- Conversation tracking ---------------------------------------------------
record_conversations               = true
conversation_idle_gap_secs         = 1800   # 30 min of silence ends a session
conversation_turn_excerpt_max_chars = 240
record_only_query_calls            = false  # true = skip fs_read / arithmetic_eval

# --- Retention / pruning -----------------------------------------------------
conversation_retention_days = 0       # 0 = keep forever
max_conversations           = 0       # 0 = unlimited
prune_on_startup            = false   # apply the two above at boot

# --- Semantic recall (optional) ----------------------------------------------
embedding_endpoint  = ""              # set to enable; e.g. http://127.0.0.1:1234/v1/embeddings
embedding_model     = "text-embedding-nomic-embed-text-v1.5"
embedding_threshold = 0.55            # cosine floor; tighten for stricter matches

# --- Auto-aliasing on semantic-only hits ------------------------------------
auto_alias_on_semantic_recall = true  # learn from semantic-only hits automatically
auto_alias_min_query_tokens   = 3     # noise guard against attaching 1-2 token queries
```

Every key has a `LODESTONE_MEMORY_<UPPER_SNAKE>` environment override
(e.g. `LODESTONE_MEMORY_AUTO_RECALL=false`,
`LODESTONE_MEMORY_CONVERSATION_RETENTION_DAYS=30`).

### Lever reference

| Knob | What turning it off / down / up does |
| --- | --- |
| `enabled` | Hides the whole family from `tools/list`; the dispatch wrapper short-circuits without touching the DB. |
| `auto_recall` | Keeps the tools available but stops the preamble. Useful when token budget is tight. |
| `recall_threshold` | Higher = quieter (only obvious matches surface); lower = chattier. |
| `recall_max_hits` | Smaller preambles when 1; richer when 5. |
| `superseded_walk_max_hops` | 0 disables the `⚠ superseded` warning entirely. |
| `record_conversations` | Keeps recall but stops growing the turn log. |
| `conversation_idle_gap_secs` | Raise to keep loosely-related sessions in one conversation; lower to split eagerly. |
| `conversation_turn_excerpt_max_chars` | Smaller = compact log; larger = richer traversal context. |
| `record_only_query_calls` | true filters silent local-system tools out of the turn log (intent-only log). |
| `conversation_retention_days` + `max_conversations` | The bulk-prune policy. Honored by `conversation_prune` (no-arg call) and by the startup sweep. |
| `prune_on_startup` | Applies the retention rules at boot. Off by default so a misconfigured policy doesn't surprise-delete on upgrade. |
| `allow_destructive` | Skips the confirm-token handshake on `*_forget` and `conversation_prune`. |
| `embedding_endpoint` | Empty = semantic recall off (no network dep). Set to enable. |
| `embedding_threshold` | Cosine floor for semantic-only hits; 0.55 is permissive, 0.65+ is strict. |
| `auto_alias_on_semantic_recall` | Off = no automatic attachment; on = system learns from every semantic-only hit. |
| `auto_alias_min_query_tokens` | Higher = more conservative about attaching short queries as phrasings. |

## Destructive tools

The following are destructive and go through the standard golden-rule-8
confirm-token guard — they return a one-time `CONFIRM` token on the first call
and do nothing; you have to call again with `confirm=<token>` (add
`trust=true` to whitelist that exact action for the session, or pre-authorize
the family with `allow_destructive = true` in the config):

- `memory_forget`
- `solution_forget`
- `conversation_forget` — deletes one conversation (CASCADE drops its turns;
  `solution_revisions.conversation_id` is set to NULL for any revision that
  referenced it, so the revision content is preserved).
- `conversation_prune` — bulk delete by retention policy
  (`older_than_days` and/or `keep_newest`). With `dry_run=true`, the count is
  reported without deleting and the confirm-token handshake is **bypassed** —
  use it to validate the policy before flipping a live prune.

`synonym_remove` and `solution_unlink` are also data-changing but small enough
to fire without the handshake — they're cheap to undo by re-adding the same
edge.

## What this means for the model

- **You almost never have to call `solution_find`.** Recall fires for free on
  every search-shaped tool. Read the preamble first, then your tool's results.
- **Treat the preamble as advisory.** Verify before reusing, and call
  `solution_update` (or `solution_link supersedes`) when you learn better.
- **If a hit is marked `⚠ superseded`, prefer the head id.** That's what the
  head walk is telling you. The obsolete record is still listed so you have
  the history, not because you should reuse it.
- **Walk the graph when the cluster matters.** A hit's outgoing links are
  printed inline; `solution_graph` and `solution_related` go further.
- **Teach the server new synonyms when you notice the same idea getting
  re-asked under different wording.** A single `synonym_add` line makes
  *every* future cache and recall robust to that phrasing change.
