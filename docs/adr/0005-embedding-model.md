# 0005 — The embedding model is chosen by measurement, pinned to the byte, and fetched by one crate

Date: 2026-09-26
Status: accepted

## Context

ADR 0002 settled that embeddings run locally through `fastembed` and ONNX Runtime, and left the model to be chosen "by measurement on a real corpus, not picked from a leaderboard", with multilingual capability verified: a meaningful share of what nooma indexes is Russian, and much of the rest mixes Russian and English in one note.

Three questions were open:

1. **Which model.** Three multilingual candidates run on ONNX Runtime at a size a laptop can carry: `intfloat/multilingual-e5-small` (118M parameters, 384 dimensions), `sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2` (118M, 384) and `BAAI/bge-m3` (568M, 1024). English-only models were ruled out before measuring.
2. **What a model is.** A repository on the Hugging Face Hub can be changed by a new commit at any time. A vector computed by one set of weights means nothing next to one computed by another, and a store of vectors silently mixed from two would rank by noise.
3. **Where the network is.** The product's promise is that no byte of a person's files leaves the machine, and that fetching the model is the one network call. `fastembed` can download models itself, through `hf-hub`, on first use - which would put an HTTP client inside the library that reads the files, and a download on the path of an ordinary command.

## Decision

**The model is `multilingual-e5-small`.** Measured with `nooma eval` on a synthetic bilingual corpus in the repository (32 questions against answers, near misses and decoys that share words) and on a private archive of 936 notes and documentation pages, 12,889 chunks, with seventy questions. MRR on the private archive: full-text 0.26, `multilingual-e5-small` 0.39, `paraphrase-multilingual-MiniLM-L12-v2` 0.28. It is the best of the fast models there, and best where full-text fails within a language - a paraphrased question (hit@3 0.72 against 0.48) and a question carrying an identifier (0.75 against 0.42). Across languages it scores zero on the private archive: it ranks a near miss in the question's language above the answer in the other. That is recorded as the price of the choice, and cross-language search is its own step (v0.9.0), not something this model was assumed to do. On four threads of a laptop CPU it reads 3-8 passages a second.

**A model is pinned to the byte.** The catalogue in `nooma-core` names each model by the commit of its repository and the size and SHA-256 of every file taken from it. A download that differs is refused. A new revision is a new id, never the same id with other weights.

**Each model keeps its own vectors.** A library stores vectors per model id, beside the full-text index, keyed by the hash of the passage the model read. A second model is a second store, not a replacement - which is what lets `nooma eval` compare two over the same library, and what makes changing the model later a matter of computing a new store rather than of trusting an old one.

**Fetching is a crate of its own, and a command of its own.** `nooma-fetch` holds the only HTTP client in the workspace. `nooma-core` loads a model from files it is given and never downloads; `fastembed` is built without `hf-hub`. A model that is not on disk is an error naming `nooma model fetch`. A test asks cargo for `nooma-core`'s dependency graph, with the model runner switched on, and fails if an HTTP client or a TLS stack appears in it.

**The model runner is a feature of `nooma-core`, off by default.** `nooma` and the window turn it on. A consumer that reads repositories - `rigger` - does not link an inference runtime it never calls.

## Consequences

Positive:

- The model on a machine is the model that was measured, and cannot drift under the same name.
- The offline promise is a property of the dependency graph, checked on every test run, not a line in a README.
- Quality is measured the same way it was chosen: `nooma eval` runs any query set against the full-text index and any model, on anyone's own folders.
- Switching models later - a better multilingual model will come - costs one catalogue entry and one pass of computing vectors, and can be compared before it is switched.

Negative:

- The first use of search by meaning needs an explicit fetch of about half a gigabyte.
- ONNX Runtime is linked statically. The binary grows by tens of megabytes, and on Windows the prebuilt library requires the C++ runtime of Visual Studio 2022 17.10 or newer to link.
- The tests run the real model and do not skip without it, so a fresh checkout fetches it once before `cargo test`, and CI caches it.

Rejected alternatives:

- **Let `fastembed` download on first use.** Simplest, and it would put a network client in the crate that reads files and a silent half-gigabyte download on the path of a search.
- **Load models by name from the Hub cache.** Convenient, and the weights could change under the name with nothing to notice it.
- **`bge-m3`.** The strongest on the synthetic set (MRR 0.87 against 0.63), and twenty times slower: 0.6 passages a second, 233 ms to turn a question into a vector, 2.3 GB. A two-thousand-note vault would take most of a day to read, which a tool running beside its owner's work cannot ask. It stays in the catalogue for measurement.
- **`paraphrase-multilingual-MiniLM-L12-v2`.** The fastest (12-13 passages a second) and the only one of the fast two that crosses languages at all (MRR 0.21-0.23 across, on the private archive), but worse than plain full-text within a language (0.27 against 0.49 on Russian), and it reads only the first 128 tokens of a chunk.
