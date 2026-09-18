---
title: repo
description: Build and read the repository index - symbols, imports and module dependencies.
---

`nooma repo` reads a git repository and answers what is in it: which files, which symbols, which files import which. It is the whole of what nooma does today. Every subcommand below is real; nothing here is planned.

`--json` exists because [`rigger`](https://github.com/lacodda/rigger) reads this before nooma speaks MCP, and a CLI with machine output is the line's standard way for one product to ask another a question. What `--json` prints is a contract: fields are added, never renamed or dropped without a format bump.

## Arguments every subcommand takes

```
nooma repo <index|symbols|deps|status> [PATH] [--json] [--store <DIR>]
```

| Argument | Meaning |
| --- | --- |
| `PATH` | The repository to read; any path inside it will do. Positional, defaults to `.`. |
| `--json` | Print one line of machine-readable JSON instead of a report. |
| `--store <DIR>` | Keep the index in this directory instead of the user's data directory. |

Every report meant for a person goes to stderr and every JSON document to stdout, so `nooma repo … --json > file` yields a file that parses.

## What a revision is

An index describes a **revision**: the commit that was checked out, and whether the working tree still matched it.

A commit alone cannot say *"the tree at this commit plus three unsaved edits"*, and an index that called that state current would be telling you your own newest work is already indexed. So both are recorded, and a revision with edits prints as `29028610+dirty`.

`dirty` is worked out by comparing content hashes against what the commit holds, not by asking `git status`. A file written since git last looked is changed by every measure that matters here and by none that git reports yet.

## `repo index`

```
nooma repo index [PATH] [--json] [--store <DIR>] [--force]
```

Brings the index up to date and stores it. A file whose bytes hash to what the index already recorded is not parsed again, so a second pass over an unchanged repository costs reading and hashing rather than parsing - on a repository of 215 files, 108 ms against 2.5 seconds.

`--force` parses every file regardless of what the hashes say. It is the way to rebuild after something outside the index changed, and a way to check that reuse gives the same answer as parsing.

```console
$ nooma repo index
29028610: 3 files, 4 symbols
parsed 3 files (3 added); reused 0
stored at C:\Users\you\AppData\Roaming\lacodda\nooma\data\index\5d50076687f7bd4e.json

$ nooma repo index
29028610: 3 files, 4 symbols
nothing changed; reused 3 files
stored at C:\Users\you\AppData\Roaming\lacodda\nooma\data\index\5d50076687f7bd4e.json
```

With `--json`:

```json
{
  "root": "C:\\Projects\\sample",
  "commit": "29028610740cc923baf966791b1e2090597fa546",
  "dirty": false,
  "files": 3,
  "symbols": 4,
  "added": 0,
  "changed": 0,
  "unchanged": 3,
  "removed": 0,
  "stored_at": "C:\\Users\\you\\AppData\\Roaming\\lacodda\\nooma\\data\\index\\5d50076687f7bd4e.json"
}
```

| Field | Meaning |
| --- | --- |
| `root` | The work tree the index describes. |
| `commit` | The commit it is pinned to, as full hex. |
| `dirty` | Whether an indexed file differed from what that commit holds. |
| `files` | How many source files are in the index. |
| `symbols` | How many symbols they declare in total. |
| `added` | Files the index had never seen. |
| `changed` | Files whose bytes no longer hashed to what was recorded. |
| `unchanged` | Files whose entries were reused without parsing. |
| `removed` | Files the index held that are no longer in the tree. |
| `stored_at` | The file the index was written to. |

`added + changed` is how many files went to the parser. The index is written only when it differs from what is stored, so a pass that changes nothing leaves the file's timestamp alone.

## `repo symbols`

```
nooma repo symbols [PATH] [--json] [--store <DIR>] [--no-refresh]
                   [--name <TEXT>] [--kind <KIND>] [--under <PREFIX>]
```

Lists what the repository declares. The index is brought up to date first, and what that cost is reported on stderr - a question about a repository should answer about the repository as it is now, and reparsing only what changed is cheap enough that refusing would be the greater surprise.

```console
$ nooma repo symbols --name ledger
src/ledger.rs:1  type  Ledger

$ nooma repo symbols --kind function --under src/
src/app.ts:2  function  run
src/ledger.rs:2  function  Ledger::post
```

| Flag | Meaning |
| --- | --- |
| `--name <TEXT>` | Only symbols whose name contains this text, compared without case. |
| `--kind <KIND>` | Only symbols of this kind: `function`, `type`, `module` or `constant`. |
| `--under <PREFIX>` | Only symbols in files whose path starts with this. |
| `--no-refresh` | Answer from the stored index without bringing it up to date. |

The filter is `--under`, not `--path`: the repository itself is the positional `PATH` argument, and two arguments sharing a value name parse as the same argument given twice - clap refuses the whole command rather than guess which one was meant.

With `--json`:

```json
{
  "commit": "29028610740cc923baf966791b1e2090597fa546",
  "dirty": false,
  "symbols": [
    {
      "path": "src/ledger.rs",
      "language": "rust",
      "name": "Ledger",
      "kind": "type",
      "line": 1,
      "parent": null
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `path` | Relative to the repository root, with `/` separators on every platform. |
| `language` | `rust`, `typescript`, `python` or `go`. |
| `name` | The identifier as it is declared. |
| `kind` | `function`, `type`, `module` or `constant`. |
| `line` | 1-based, as an editor counts lines. |
| `parent` | The enclosing symbol's name, or `null` at file scope. |

## `repo deps`

```
nooma repo deps [PATH] [--json] [--store <DIR>] [--no-refresh]
```

Shows which files import which, within this repository. Only imports that resolve to an indexed file are edges; an import of `react` or `std::fmt` stays a fact about the file rather than becoming an edge to nothing.

```console
$ nooma repo deps
src/app.ts
  -> src/card.ts
```

With `--json`:

```json
{
  "commit": "29028610740cc923baf966791b1e2090597fa546",
  "dirty": false,
  "dependencies": {
    "src/app.ts": ["src/card.ts"]
  }
}
```

| Field | Meaning |
| --- | --- |
| `commit` | The commit the answer describes. |
| `dirty` | Whether the tree had edits when it was indexed. |
| `dependencies` | Each file that has resolved imports, mapped to the files it imports. A file with no resolved imports is absent rather than present with an empty list. |

A repository of pure Rust usually yields an empty graph, and that is correct: Rust's `use` paths are not relative, so none of them can be resolved to a file without guessing.

## `repo status`

```
nooma repo status [PATH] [--json] [--store <DIR>]
```

Says what the stored index describes, without changing it. The only subcommand that never refreshes: it is the question to ask before deciding whether to do anything, and a question that changes what it asks about cannot be asked twice.

```console
$ nooma repo status
current at 29028610: 3 files, 4 symbols

$ nooma repo status
indexed at 29028610+dirty with uncommitted edits: 3 files, 4 symbols

$ nooma repo status
stale: indexed at 29028610, checked out 4fa18bad — run `nooma repo index`
```

An index of a tree with uncommitted edits is not `current`, and no command makes it so - the edits are there until they are committed. It says what it is rather than prescribing a rebuild that would change nothing.

With `--json`:

```json
{
  "root": "C:\\Projects\\sample",
  "commit": "29028610740cc923baf966791b1e2090597fa546",
  "indexed": true,
  "current": true,
  "reusable": true,
  "indexed_commit": "29028610740cc923baf966791b1e2090597fa546",
  "indexed_dirty": false,
  "files": 3,
  "symbols": 4
}
```

| Field | Meaning |
| --- | --- |
| `root` | The work tree. |
| `commit` | The commit currently checked out. |
| `indexed` | Whether a stored index exists at all, for any revision. |
| `current` | Whether it can be used as-is: same commit, clean tree, current rules. |
| `reusable` | Whether its entries can be reused at all, or were cut up by older rules. |
| `indexed_commit` | The commit the stored index describes, or `null` if there is none. |
| `indexed_dirty` | Whether the tree had edits when it was indexed, or `null`. |
| `files` | How many files it holds, or `null`. |
| `symbols` | How many symbols it holds, or `null`. |

`current` and `reusable` answer different questions. An index cut up by older rules describes its commit perfectly well and still has to be rebuilt, so a caller reading only `current` would skip a rebuild it needs.

## When an index is thrown away

Two numbers are stored beside the entries, and each one protects against a different way of being quietly wrong.

- **The format version** covers the file's shape. A reader that meets a different number refuses the file rather than reading it under today's meaning, and says to reindex.
- **The chunker version** covers what an entry means. Changing a tree-sitter query or how a file is divided changes what every stored entry says while the file's bytes - and therefore its hash - stay exactly the same. Without a number to compare, entries built by the old rules would be kept and extended by the new ones, and the result reads as search quietly getting worse.
