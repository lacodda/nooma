# 0001 — Hybrid index: full-text and vector search from the first release

Date: 2026-08-09
Status: accepted

## Context

The obvious design for a semantic search tool is a single vector index: chunk the documents, embed the chunks, embed the query, return nearest neighbours.

That design fails on a large class of real queries. Embeddings capture meaning and discard exact tokens, so a query containing an invoice number, an error code, a person's surname or a file identifier returns semantically adjacent documents rather than the one document that literally contains the string. Users do not experience this as "the model generalizing" — they experience it as the tool being broken, because they know the file exists.

The inverse is equally true: full-text search cannot answer a query phrased as a description.

## Decision

Two indexes are built and queried together from v0.1.0 onward:

- `tantivy` for full-text retrieval — mature, 16M downloads, handles exact tokens and phrases.
- `usearch` for approximate nearest-neighbour search over embeddings.

Results are fused into a single ranked list (reciprocal rank fusion as the starting point, with the balance exposed as a setting).

The full-text half ships first because it is independently useful, the vector half follows, and the fusion is the MVP boundary: neither half alone constitutes the product. The version numbers moved when the repository index was put first (see 0003) — the full-text half lands in v0.5.0, the model is chosen in v0.6.0, and the hybrid is v0.8.0 — but the ordering and the boundary are unchanged.

## Consequences

Positive:

- Both query styles work: descriptions and exact identifiers.
- The full-text index gives a fast first response while the query embedding is still being computed, which reads as low latency.
- The full-text half is shippable and useful before any model is chosen.

Negative:

- Two indexes to build, persist, invalidate and keep consistent.
- Fusion needs tuning, and tuning needs an evaluation set of queries with known-good answers.
- Storage roughly doubles compared with a single index.

Rejected alternatives:

- **Vector index only** — simpler and more impressive in a demo, but it silently loses exact-match queries, which is the failure mode users notice first.
- **Full-text only** — reliable, but it is the thing that already exists and does not solve the stated problem.
