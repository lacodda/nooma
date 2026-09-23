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

There is exactly one exception, and it is not in the default build: `--prose` can add a generated paragraph saying what a module is for, by asking the [Claude Code](https://claude.com/claude-code) CLI you installed, under your own subscription. It needs `--features prose` to exist at all, `--prose` to run, and it labels everything it writes. The released binaries are default builds, so the code is not in them: they answer `--prose` with `error: unexpected argument`, which is what a flag compiled out looks like.

## What works today

v0.5.0 searches your folders by the words in them - the exact half of the hybrid. Point it at notes, text files or an Obsidian vault, and search from a window with one field or from the shell:

<p align="center"><img src="https://github.com/lacodda/nooma/raw/main/assets/screenshot.png" alt="The nooma window: one field in the title bar, results with the matched words marked" width="720"></p>

```
$ nooma source add ~/Notes
$ nooma find "договоров"
Договоры с подрядчиками · Сроки
  ~/Notes/work/договоры.md:9
  Договор поставки продлевается автоматически…
```

Any form of a word finds the others, in Russian and English alike. Tags, wikilinks and backlinks count; only files that changed are read again.

The repository index from earlier versions is still here: `nooma repo` reads a git repository into symbols, imports, module summaries and commit history, for [rigger](https://github.com/lacodda/rigger) and for scripts.

## Status

v0.5.0. Exact search over markdown and text works, in a window and on the command line; the half that finds a document by meaning arrives with the embedding model, and the two merge into one ranked list at v0.8.0 - the point at which the name is earned. PDF, DOCX and EPUB come after. See the [changelog](https://github.com/lacodda/nooma/blob/main/CHANGELOG.md) for what has shipped, and [CONTRIBUTING.md](https://github.com/lacodda/nooma/blob/main/CONTRIBUTING.md) for the build.

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
