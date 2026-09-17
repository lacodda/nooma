# Contributing to nooma

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
