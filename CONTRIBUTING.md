# Contributing to nooma

## Layout

The repository is a cargo workspace of two crates:

- **`crates/nooma-core`** — the indexing library. No UI, no window, no network path. This is where the work is.
- **`crates/nooma`** — the binary, and the first caller of the library.

They share one version. `nooma-core` is not versioned apart from the product it belongs to.

## Building

```
cargo build --release
cargo run -- repo index
```

The gate, which every commit has to pass:

```
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && cargo build --release
```

Requires Rust 1.95 or newer. That number is a promise to anyone building from source, not a note about the maintainer's machine: the `msrv` job in CI builds on exactly the version the manifest declares, so raising a toolchain does not quietly raise the floor.

## Adding a language

A language is one row of the table in `crates/nooma-core/src/lang.rs` plus two tree-sitter queries under `src/queries/<language>/`. Nothing else in the crate needs to know about it.

The queries follow a convention:

| Capture | Means |
| --- | --- |
| `@name` | the identifier to record |
| `@kind.function`, `@kind.type`, `@kind.module`, `@kind.constant` | what kind of symbol this is; the capture wraps the whole declaration, which is also the span a nested declaration is tested against |
| `@scope` | a named region that is not itself a declaration — a Rust `impl` block — which lends its span for parenting and stays out of the index |
| `@module` | an import's module path, as written |

Three ways a query goes wrong while still compiling, all of them found the hard way:

- **A predicate outside the pattern's parentheses** is a separate pattern that applies to nothing. `(pattern) (#eq? @x "y")` compiles clean and silently means something else; `((pattern) (#eq? @x "y"))` is the form that works.
- **A `@kind` capture on too wide a node** makes everything below it a child of that symbol. Hang it on the narrowest node that still covers the declaration's body.
- **Two patterns matching one node both fire** — tree-sitter does not fall through to a later pattern. Where a query cannot express the exclusion, duplicates are folded in `symbols.rs` with an explicit precedence.

Add fixtures to `crates/nooma-core/tests/extraction.rs` for every kind the new language declares. A query that compiles proves the node names exist, not that they are the ones a declaration uses.

## Testing

Fixtures are written for the test. Nothing in the test corpus comes from a real repository or a real archive: nooma indexes personal files, and no fragment of real content belongs in the code, the tests, the fixtures or the screenshots.

Green tests are not a verified product. Three defects in v0.1.0 were found only by running the binary against real repositories while 39 tests passed — a Windows path prefix leaking into every output, a type reported once per `impl` block, and a CLI flag that clashed with a positional argument so that no invocation parsed. Run it on something real before calling a version done.

## Principles

**Nothing goes out.** Downloading the embedding model is the only network call, and it is explicit and one-time. There is no cloud mode, not even opt-in.

**Hybrid from the start.** Exact and semantic search are two halves of one feature, not two milestones.

**Indexing stays out of the way.** Background threads at low priority with a CPU ceiling.

**A changed index format is a breaking change.** It ships with a migration path in the changelog, or with a forced reindex the user is told about. An index that is silently misread looks like "search stopped finding things", which is the worst failure this product has.

Architecture decisions are recorded in [`docs/adr/`](docs/adr/).
