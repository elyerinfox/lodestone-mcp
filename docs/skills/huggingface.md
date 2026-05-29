# Hugging Face Hub — `hf_model_search` / `hf_dataset_search` / `hf_model`

|  |  |
| --- | --- |
| **Module** | [`src/skills/huggingface.rs`](../../src/skills/huggingface.rs) |
| **Tools** | `hf_model_search`, `hf_dataset_search`, `hf_model` |
| **Network** | keyless API |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Searches and inspects the Hugging Face Hub through its public JSON endpoints
(`huggingface.co/api`), with no token — a token would only be needed for
private/gated repos. `hf_model_search` and `hf_dataset_search` each search one
corpus (no hidden mode flag — pick the tool that matches what you want);
`hf_model` returns one model's metadata. Results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `hf_model_search` | `query`, `max_results?` | Search the Hub for **models** by name or keyword. Returns id, link, downloads, likes, and task/pipeline, sorted by downloads (descending). Default 10 results, capped at 25. |
| `hf_dataset_search` | `query`, `max_results?` | Search the Hub for **datasets** by name or keyword. Returns id, link, downloads, likes, sorted by downloads (descending). Default 10 results, capped at 25. |
| `hf_model` | `model` | Fetch one model's metadata: downloads, likes, task/pipeline, library, license, last-modified date, and topic tags. `model` is a model id like `google-bert/bert-base-uncased` or `gpt2`. |

## Configuration & gating
No configuration. All three tools are on by default with no tunables; disable them
in `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)).

## Example uses
- **Find then inspect a model** — `hf_model_search` for "bert" to discover top models by downloads, then `hf_model` with `model="google-bert/bert-base-uncased"` for its license, library, and tags.
- **Browse datasets** — `hf_dataset_search` with `query="squad"` to list matching datasets with download/like counts.
- **Check a model's license before use** — `hf_model` with `model="gpt2"` to read its license tag and pipeline task.

## See also
- [tools.md](../tools.md) — full tool reference (Retrieve section).
- [arxiv.md](arxiv.md) — papers behind many models (model tags often include arXiv ids).
