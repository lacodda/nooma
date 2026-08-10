# 0002 — Embeddings run locally through ONNX Runtime

Date: 2026-08-09
Status: accepted

## Context

Semantic search needs an embedding model. Three ways to get one:

1. A cloud embedding API — highest quality, trivial to integrate, and it sends the content of every indexed file to a third party. For a personal archive this is disqualifying, and it is the exact reason the product exists.
2. `candle` — pure Rust, works with any Hugging Face model, no ONNX conversion needed.
3. ONNX Runtime via `fastembed` — a fixed catalogue of models, converted to ONNX.

The relevant measurement: on `all-MiniLM-L12-v2`, the Candle path reaches 5–11 documents per second while the ONNX Runtime path reaches 70–230. Roughly 14×. For a vault of several thousand files that is the difference between indexing over lunch and indexing over a weekend.

## Decision

Embeddings are computed locally through `fastembed`, which wraps ONNX Runtime. The model is downloaded once on first use and cached; afterwards the product functions with no network access whatsoever.

No file content is transmitted anywhere, under any configuration. There is no opt-in cloud mode.

The specific model is chosen by measurement on a real corpus before v0.2.0, not picked from a leaderboard. Multilingual capability must be verified — a meaningful share of the target corpus is Russian.

## Consequences

Positive:

- Indexing is fast enough that first-run indexing is tolerable.
- The privacy claim is structural rather than a policy promise: there is no code path that sends content out.
- Works on a machine with no internet after the initial model fetch.

Negative:

- Limited to models available in ONNX form through the `fastembed` catalogue.
- Quality is below the best cloud embedding models.
- The ONNX Runtime dependency adds native library weight to the build.

Rejected alternatives:

- **Cloud embedding API** — best quality, unacceptable premise.
- **`candle`** — more model freedom and pure Rust, but 14× slower on CPU, which pushes first-run indexing from hours into days.
