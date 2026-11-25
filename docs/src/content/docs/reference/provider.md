---
title: provider
description: The FullText trait of nooma-core - one folder searched by its words and by its meaning, for another program.
---

A program that works on a folder of notes - an editor, a notebook - keeps its own index of what the notes *are*: names, headings, links, tags. It has no reason to keep a second one of what they *say*. `nooma-core` is the library nooma itself is built on, and `provider` is the part of it such a program asks.

Nothing here opens a network connection. The crate is the one the [offline test](https://github.com/lacodda/nooma/blob/main/crates/nooma-core/tests/offline.rs) holds to that.

## Taking the crate

```toml
[dependencies]
nooma-core = { git = "https://github.com/lacodda/nooma", tag = "v0.17.0" }
```

That is the half by words alone: no inference runtime, no C++ library, a build of a couple of minutes. The half by meaning is the `semantic` feature:

```toml
nooma-core = { git = "https://github.com/lacodda/nooma", tag = "v0.17.0", features = ["semantic"] }
```

## The question: `FullText`

```rust
pub trait FullText {
    fn update(&self) -> Result<Updated>;
    fn search(&self, query: &str, limit: usize) -> Result<Answer>;
    fn explain(&self, query: &str, hit: &mut Hit) -> Result<bool> { Ok(false) }
    fn standing(&self) -> Result<Standing>;
}
```

| Method | What it does |
| --- | --- |
| `update` | Brings what is known of the folder up to date with its files. Only the files that changed are read. |
| `search` | The documents that answer a query, best first, one row per document. From what the last update read: a search never reads the folder. |
| `explain` | Points a row the meaning found at the sentence of its chunk that answered. Apart from the search because it costs a reading of that chunk's sentences; a provider that cannot say leaves the row as it was and returns `false`. |
| `standing` | What it holds and what it can answer by, without changing anything. |

It is a trait because nooma's is not the only answer. A search that reads the files line by line each time is another, and the one a program falls back on - so the program holds a `Box<dyn FullText>` and does not ask which it has.

`Answer` is what [`nooma find --json`](/nooma/reference/find/) prints: rows with the file, the headings above the chunk, its line, a fragment with the matching words marked, and where each half put the document. One list, ranked [from both halves](/nooma/concepts/hybrid/).

## nooma's answer: `Folder`

```rust
use nooma_core::{Folder, FullText};

let notes = Folder::open(vault, store, vec![".trash/".into()])?;
notes.update()?;
for row in notes.search("how to descale the kettle", 10)?.hits {
    println!("{}:{}  {}", row.hit.path.display(), row.hit.line, row.hit.fragment);
}
```

`Folder::open(folder, store, exclude)` takes the folder to search, a directory of the caller's own to keep the index in, and patterns to leave out in `.gitignore` syntax. Nothing is read until the first `update`. The folder is read as [nooma reads a source](/nooma/reference/find/#index): Markdown and text, and PDF, Word documents and books where they lie among the notes - leave them out by pattern if they are not wanted.

A store answers for one folder. Opened for another, or with other exclusions, it follows, and the documents that no longer belong leave at the next update.

| Method | What it does |
| --- | --- |
| `Folder::open` | The provider for a folder, by words. |
| `with_model(models)` | Answer by meaning too, with the model nooma uses, when it is in `models` - the directory [`nooma model fetch`](/nooma/reference/model/) put it in, `nooma_core::model::default_models_dir()`. Without the model there, the provider is returned as it was. `semantic` only. |
| `with_embedder(embedder)` | Answer by meaning too, with any `Embedder`. `semantic` only. |
| `update_with(progress)` | `update`, telling how far it has got: `Step::Files`, then `Step::Passages` while the model reads. |
| `folder`, `library` | The folder, and the library underneath for what the trait does not ask - a preview, the health of the files. |

The model is never fetched by the crate. A program that wants the half by meaning points `with_model` at the model nooma fetched, and answers by words until it is there; `standing().meaning` says which.

### While the vectors are computed

The first update with a model reads every passage of the folder, a few a second: minutes for a notebook, an hour for a large vault. Run it on a thread of its own. A search asked meanwhile is answered - the update borrows the model a batch at a time and steps aside for a search that is waiting - and covers what has been read so far: `standing().meaning.behind` counts the documents the vectors do not cover yet, and those are found by their words.

`update` fails with `Error::Busy` when another process is updating the same store. What is stored still answers.

## What stands behind it

| Type | Meaning |
| --- | --- |
| `Updated` | `files` - the library's report of what was read; `embedded` - passages the model read, none without a model. |
| `Standing` | `documents`, `indexed_at`, `stale` - why nothing can be found until the next update, when the stored index was written by other rules - and `meaning`. |
| `Meaning` | `model`, `ready` - whether there are vectors to answer from - and `behind`. `None` in `Standing` when the provider answers by words alone. |

Why a trait, why one store per folder and why the model is the caller's to attach is in [ADR 0014](https://github.com/lacodda/nooma/blob/main/docs/adr/0014-a-provider-for-neighbours.md).
