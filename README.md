<p align="center"><img src="https://github.com/lacodda/nooma/raw/main/assets/banner.svg" alt="nooma - local search by meaning" width="720"></p>

> Local search that finds a file by what it is about. Nothing leaves your machine.

<p align="center">
  <a href="https://github.com/lacodda/nooma/actions"><img src="https://img.shields.io/github/actions/workflow/status/lacodda/nooma/ci.yml?style=flat-square" alt="CI"></a>
  <a href="https://github.com/lacodda/nooma/blob/main/LICENSE"><img src="https://img.shields.io/github/license/lacodda/nooma?style=flat-square" alt="License"></a>
</p>

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

There is exactly one exception, and it is not in the default build: `--prose` can add a generated paragraph saying what a module is for, by asking the [Claude Code](https://claude.com/claude-code) CLI you installed, under your own subscription. It needs `--features prose` to exist at all, `--prose` to run, and it labels everything it writes. A default build has no such code path in it — check with `strings` rather than taking our word for it.

## What works today

v0.4.0 ships the first half: `nooma-core`, a library that reads a git repository into symbols, imports and module dependencies, and the `nooma repo` command over it.

```
$ nooma repo index
indexed 47711d3c: 13 files, 159 symbols

$ nooma repo symbols --name ledger --kind type
src/ledger.rs:12  type  Ledger

$ nooma repo summary --under src/ledger.rs
src/ledger.rs
  Keeps entries in balance.
      4  pub fn post(entry: Entry) -> Result<Balance>

$ nooma repo history --matching "path handling"
4fa18bad  fix: correct the PATH handling on Windows
```

Built with `--features prose`, `nooma repo summary --prose` adds a generated paragraph per module, marked as generated and cached so each distinct file is described — and paid for — once.

Rust, TypeScript, Python and Go, parsed with tree-sitter. The index is kept on disk and describes a revision - the commit, and whether the tree still matched it. Only files whose contents actually moved are parsed again, so a second pass over 287 unchanged files takes 165 ms rather than 4.5 seconds. `.gitignore` and `.nooma-ignore` decide what is left out.

This half is built first because [rigger](https://github.com/lacodda/rigger) needs it, and because an index is easier to get right before a window depends on it. Search over documents — and the hybrid the product is named for — comes next.

## Status

v0.4.0. The repository index works — symbols, module summaries, optional generated prose, and commit messages as documents; document search does not exist yet, and nothing here can be pointed at a folder of notes today. The hybrid over documents is the point at which nooma becomes useful to a person rather than to another program. See the [changelog](https://github.com/lacodda/nooma/blob/main/CHANGELOG.md) for what has shipped, and [CONTRIBUTING.md](https://github.com/lacodda/nooma/blob/main/CONTRIBUTING.md) for the build.

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
