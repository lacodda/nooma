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

v0.11.0 searches your folders two ways at once - by the words in them and by what they mean - and answers with one list, in Russian and English alike. Point it at notes, text files, PDFs, Word documents, books or an Obsidian vault, and search from a window with one field or from the shell:

<p align="center"><img src="https://github.com/lacodda/nooma/raw/main/assets/screenshot.png" alt="The nooma window: one field in the title bar and one list of results, each marked by the half that found it - a PDF at its page and bookmark first; Russian notes found for an English question are marked another language, with the sentence that answered highlighted" width="720"></p>

```
$ nooma source add ~/Notes
$ nooma index
$ nooma find "espresso machine warranty" --limit 3
Appliance receipts · March (words + meaning 0.90)
  ~/Notes/home/appliance-receipts.md:5
  Filed the warranty claim for the espresso machine on 14 March…

Гарантийный ремонт (meaning 0.88, another language)
  ~/Notes/home/гарантия.md:3
  Кофемашину приняли в сервис по гарантии: понадобились чек и серийный номер…

Kitchen appliances · Espresso setup (meaning 0.83)
  ~/Notes/home/kitchen-appliances.md:5
  …the seller promises a replacement part under guarantee.
```

Any form of a word finds the others; the meaning finds the notes in words they do not use - a guarantee, a repair - and in the other language, showing the sentence that answered. Each document appears once, marked by the halves that found it; on the author's own archive the one list beats either half alone. Paste a paragraph into the window - or give it to `nooma similar` - to find what else says the same.

A PDF is read page by page: its result names the page the answer is on and the chapter its bookmarks give, and a word the typesetter broke across two lines is found whole. A scan has no text to read yet; it is found by its name, and the index says which files are scans.

A Word document is read under its headings, a book under its table of contents, and a book is found by its author too. A file is what its bytes say, not its name: a PDF a browser saved as `invoice` is a PDF. A note saved in Windows-1251 or KOI8-R is read in its encoding, and the index says which encodings it had to guess.

The model runs on your CPU and is fetched once, when you ask: a button in the window, `nooma model fetch` in the shell. The first reading of a large library takes an hour or so; it runs in the background, and search answers over what is read meanwhile. `nooma eval` measures each half and the list against questions whose answers you know, on your own folders.

The repository index from earlier versions is still here: `nooma repo` reads a git repository into symbols, imports, module summaries and commit history, for [rigger](https://github.com/lacodda/rigger) and for scripts.

## Status

v0.11.0. Markdown, text, PDF, Word documents and EPUB books are searched by words and by meaning and answered with one list - in a window and on the command line - and a pasted passage finds its kin: the hybrid the name promises. A question in Russian finds the English document and the other way round, and says why; a PDF is found at its page. Reading scans by OCR comes next. See the [changelog](https://github.com/lacodda/nooma/blob/main/CHANGELOG.md) for what has shipped, and [CONTRIBUTING.md](https://github.com/lacodda/nooma/blob/main/CONTRIBUTING.md) for the build.

## License

MIT (c) [Kirill Lakhtachev](https://lacodda.com)
