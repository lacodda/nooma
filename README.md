<p align="center"><img src="https://github.com/lacodda/nooma/raw/main/assets/banner.svg" alt="nooma - local search by meaning" width="720"></p>

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

Scaffold stage: the crate builds and runs, but indexing and search are not written yet — nothing here can be pointed at a folder today. See [CONTRIBUTING.md](https://github.com/lacodda/nooma/blob/main/CONTRIBUTING.md) for the build and the project's principles.

## License

MIT — see [LICENSE](https://github.com/lacodda/nooma/blob/main/LICENSE).
