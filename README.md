<!-- Banner is added in v0.8.0 together with the brand assets. -->

# nooma

**Local search that finds a file by what it is about. Nothing leaves your machine.**

You remember the meaning, not the words: *"the note about fixing permissions on that ini file"*, *"the PDF with the warranty terms"*. Windows Search wants the exact string — the one thing you have forgotten. `nooma` takes the description.

The name is Greek — *νόημα*, thought, meaning, the thing that was meant.

## Why hybrid, not just semantic

Pure semantic search demos beautifully and fails on real work. It will find *"something about losing a close friend"* and then lose *"the file mentioning invoice 7743013902"*. Exact strings, numbers and identifiers are full-text work.

So `nooma` runs two indexes and merges them:

- **`tantivy`** — full-text, for exact matches
- **`usearch`** — vector index, for meaning
- Hybrid ranking fuses both into one list

This is a condition of the product being usable, not a refinement scheduled for later.

## Offline by construction

Embeddings run locally on the CPU through ONNX Runtime. The model downloads once; after that there is no network path for your content at all. No cloud, no API keys, no telemetry — the same stance as [sefy](https://github.com/lacodda/sefy).

## Status

Early development. The version map to 1.0 is fixed:

| Version | What lands |
| --- | --- |
| v0.1.0 | Index and exact search over md/txt via `tantivy` |
| v0.2.0 | Embeddings (`fastembed`) and vector index (`usearch`) |
| v0.3.0 | Hybrid ranking — the usable milestone |
| v0.4.0 | PDF, docx, epub |
| v0.5.0 | Incremental reindexing on file changes |
| v0.6.0 | Result preview with highlighting |
| v0.7.0 | Code indexing via `tree-sitter` |
| v0.8.0 | Global hotkey, tray, installer, auto-update |
| v1.0.0 | Public release |

Image search via CLIP embeddings is planned after 1.0.

## Building

```
cargo run --release
cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

Requires Rust 1.85 or newer.

## Principles

**Nothing goes out.** Downloading the model is the only network call, and it is explicit and one-time.

**Hybrid from the start.** Exact and semantic search are two halves of one feature, not two milestones.

**Indexing stays out of the way.** Background threads at low priority with a CPU ceiling.

Architecture decisions are recorded in [`docs/adr/`](docs/adr/).

## License

MIT — see [LICENSE](LICENSE).
