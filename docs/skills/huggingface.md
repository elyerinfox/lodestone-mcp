# Hugging Face Hub — `hf_search` / `hf_model`

|  |  |
| --- | --- |
| **Module** | [`src/skills/huggingface.rs`](../../src/skills/huggingface.rs) |
| **Tools** | `hf_search`, `hf_model` |
| **Network** | keyless API |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Searches and inspects the Hugging Face Hub through its public JSON endpoints
(`huggingface.co/api`), with no token — a token would only be needed for
private/gated repos. `hf_search` lists models or datasets sorted by downloads;
`hf_model` returns one model's metadata. Results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `hf_search` | `query`, `kind?`, `max_results?` | Search the Hub by name or keyword. `kind` is `"model"` (default) or `"dataset"`. Returns id, link, downloads, likes, and task/pipeline, sorted by downloads (descending). Default 10 results, capped at 25. |
| `hf_model` | `model` | Fetch one model's metadata: downloads, likes, task/pipeline, library, license, last-modified date, and topic tags. `model` is a model id like `google-bert/bert-base-uncased` or `gpt2`. |

## Configuration & gating
No configuration. Both tools are on by default with no tunables; disable them in
`[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)).

## Example uses
- **Find then inspect a model** — `hf_search` for "bert" to discover top models by downloads, then `hf_model` with `model="google-bert/bert-base-uncased"` for its license, library, and tags.
- **Browse datasets** — `hf_search` with `query="squad"`, `kind="dataset"` to list matching datasets with download/like counts.
- **Check a model's license before use** — `hf_model` with `model="gpt2"` to read its license tag and pipeline task.

## See also
- [tools.md](../tools.md) — full tool reference (Retrieve section).
- [arxiv.md](arxiv.md) — papers behind many models (model tags often include arXiv ids).
