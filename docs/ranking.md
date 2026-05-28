# Search strategies & ranking

How lodestone turns several providers' results into one ordered list. All of this
applies to the **aggregate** strategy; **fallback** returns the first provider's
results unchanged. Implementation: [`src/provider.rs`](../src/provider.rs)
(`Registry::search`, `merge`, `composite`, `score`).

## Strategies (`[search].strategy`)

- **fallback** (default) — try the kind's providers in order; the first with a
  non-empty result set wins. Fewest requests, lowest latency, no re-ranking.
- **aggregate** — query every provider for the kind **concurrently**, dedupe by
  normalized URL, then re-rank with the configured method below. Broader coverage
  at the cost of more requests.

Both are settable **per kind** (`[search.web]` / `[search.code]` / `[search.qa]`),
inheriting the global value when a field is empty — e.g. aggregate web/code for
coverage while keeping qa on fallback so the StackExchange API isn't hit on every
query.

## Dedup

Before ranking, results are merged by **normalized URL** (`normalize_url`: drop
the `#fragment` and any trailing `/`). A merged entry remembers every
`(engine, rank)` that produced it (its *sources*), keeps the longest snippet, and
backfills a missing title. The `meta` field records `found by: <engines>`.

## Ranking methods (`[search].ranking`)

| Method | Idea |
| --- | --- |
| **`composite`** (default) | Multi-signal fusion + diversification (below). |
| `reciprocal` | Σ `1/(rank+1)` over sources — placement + agreement. |
| `borda` | Σ `(N − rank)` — linear positional scoring. |
| `breadth` | Consensus: number of engines that returned it, best position breaks ties. |
| `interleave` | Round-robin: each engine's 1st, then 2nd, … — maximum source diversity. |

Any method can be set per kind, same as strategy.

## The composite ranker

The default. Each deduped result gets a **base score** that is the product of four
factors, after which results are **domain-diversified** during selection. The aim:
relevant *and* corroborated *and* trustworthy *and* non-redundant — more than a
weighted-position sum alone.

```
base = rrf × consensus × lexical × authority
```

**1. Weighted Reciprocal Rank Fusion (RRF).** The backbone.

```
rrf = Σ over sources  weight(engine) / (RRF_K + rank)        RRF_K = 60
```

The canonical `k ≈ 60` damps the dominance of any single engine's rank-0 hit,
which makes fusion across engines far more stable than `1/(rank+1)`.
`weight(engine)` defaults to `1.0` and is overridable per engine (see tuning).

**2. Consensus.** Rewards results multiple engines agree on.

```
consensus = 1 + 0.25 × (distinct_engines − 1)
```

**3. Lexical relevance.** A cheap relevance signal that position-only mergers
ignore: the fraction of distinct query terms that appear (as substrings) in the
result's `title + snippet` (lowercased).

```
lexical = 1 + 0.5 × coverage          coverage ∈ [0, 1]
```

Query terms are the query's whitespace tokens, lowercased and stripped to
alphanumerics, dropping operators (anything containing `:`, e.g. `site:`) and
tokens shorter than 2 chars. An empty query contributes a neutral factor of 1.

**4. Authority.** Small additive trust signals (`authority = 1 + a`):

| Signal | + |
| --- | --- |
| URL is `https://` | 0.05 |
| Trusted domain (built-in set ∪ `trusted_domains`) | 0.15 |
| Resolved code hit (`repo` set) | 0.05 |
| Q&A votes | `min(votes, 100) / 100 × 0.3` |

Built-in trusted domains: `stackoverflow.com`, `developer.mozilla.org`,
`docs.rs`, `doc.rust-lang.org`, `rust-lang.org`, `github.com`, `docs.python.org`,
`pkg.go.dev`, `kubernetes.io`, `wikipedia.org`, `man7.org` (subdomains match too).

**5. Domain diversification (MMR).** Selection is greedy: repeatedly take the
highest *effective* score, where a domain already chosen `n` times is decayed:

```
effective = base × 0.6ⁿ
```

so one site can't monopolize the top results — broadening the result set without
dropping anything outright.

Tuning constants live in `src/provider.rs` (`RRF_K`, `CONSENSUS_BONUS`,
`LEXICAL_WEIGHT`, `DIVERSITY_DECAY`).

## Tuning (config)

```toml
[search]
ranking = "composite"            # composite | reciprocal | borda | breadth | interleave
trusted_domains = ["docs.internal.corp"]   # extra authority-boosted domains

[search.engine_weights]          # per-engine weight in the RRF term (default 1.0)
duckduckgo = 1.0
mojeek     = 0.8
```

Env: `LODESTONE_SEARCH_RANKING` (the weights/trusted lists are file-only). See
[`config/02-search.toml`](../config/02-search.toml).

## How this differs from SearXNG

SearXNG merges with a weighted sum of reciprocal *positions* and groups identical
results. Composite keeps that fusion as its backbone (with the proven RRF
constant) but additionally factors in **lexical relevance** and **authority**, and
then **diversifies by domain** — so the ranking is both more relevant and less
redundant out of the box. Engine weighting (SearXNG's signature knob) is supported
via `[search.engine_weights]`.
