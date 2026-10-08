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

Embeddings run locally on the CPU through ONNX Runtime. The model downloads once, when you run `nooma model fetch`, pinned to the byte; after that there is no network path for your content at all - the library that reads your files has no HTTP client among its dependencies, and a test holds it to that. No cloud, no API keys, no telemetry — the same stance as [sefy](https://github.com/lacodda/sefy).

There is exactly one exception, and it is not in the default build: `--prose` can add a generated paragraph saying what a module is for, by asking the [Claude Code](https://claude.com/claude-code) CLI you installed, under your own subscription. It needs `--features prose` to exist at all, `--prose` to run, and it labels everything it writes. The released binaries are default builds, so the code is not in them: they answer `--prose` with `error: unexpected argument`, which is what a flag compiled out looks like.

## What works today

v0.8.0 searches your folders two ways at once - by the words in them and by what they mean - and answers with one list. Point it at notes, text files or an Obsidian vault, and search from a window with one field or from the shell:

<p align="center"><img src="https://github.com/lacodda/nooma/raw/main/assets/screenshot.png" alt="The nooma window: one field in the title bar and one list of results, each marked by the half that found it" width="720"></p>

```
$ nooma source add ~/Notes
$ nooma index
$ nooma find "espresso machine warranty" --limit 3
Appliance receipts · March (words + meaning 0.90)
  ~/Notes/home/appliance-receipts.md:5
  Filed the warranty claim for the espresso machine on 14 March…

Kitchen appliances · Espresso setup (meaning 0.84)
  ~/Notes/home/kitchen-appliances.md:5
  …the seller promises a replacement part under guarantee.

Home insurance (meaning 0.79)
  ~/Notes/home/insurance.md:3
  Household appliances are covered for accidental damage…
```

Any form of a word finds the others, in Russian and English alike; the meaning finds the notes in words they do not use - a guarantee, an insurance claim. Each document appears once, ranked from both halves and marked by the ones that found it; on the author's own archive the one list finds the answer more often than either half alone. Paste a paragraph into the window - or give it to `nooma similar` - to find what else says the same.

The model runs on your CPU and is fetched once, when you ask: a button in the window, `nooma model fetch` in the shell. The first reading of a large library takes an hour or so; it runs in the background, and search answers over what is read meanwhile. `nooma eval` measures each half and the list against questions whose answers you know, on your own folders.

The repository index from earlier versions is still here: `nooma repo` reads a git repository into symbols, imports, module summaries and commit history, for [rigger](https://github.com/lacodda/rigger) and for scripts.

## Status

v0.8.0. Markdown and text are searched by words and by meaning and answered with one list - in a window and on the command line - and a pasted passage finds its kin: the hybrid the name promises. Search across languages is v0.9.0: the model nooma uses does not cross them yet. PDF, DOCX and EPUB come after. See the [changelog](https://github.com/lacodda/nooma/blob/main/CHANGELOG.md) for what has shipped, and [CONTRIBUTING.md](https://github.com/lacodda/nooma/blob/main/CONTRIBUTING.md) for the build.

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
