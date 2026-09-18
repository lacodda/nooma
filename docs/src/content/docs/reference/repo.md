---
title: repo
description: Build and read the repository index - symbols, imports and module dependencies.
---

`nooma repo` reads a git repository at its checked-out commit and answers what is in it: which files, which symbols, which files import which. It is the whole of what nooma does in v0.1.0. Every subcommand below is real; nothing here is planned.

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

## `repo index`

```
nooma repo index [PATH] [--json] [--store <DIR>] [--force]
```

Reads the repository at its checked-out commit and stores the index. If a stored index already describes that commit, `index` says so instead of doing the work again - `--force` rebuilds regardless.

```console
$ nooma repo index
indexed 47711d3c: 13 files, 159 symbols
stored at C:\Users\you\AppData\Local\lacodda\nooma\data\index\a1b2c3d4e5f60718293a4b5c6d7e8f90.json

$ nooma repo index
already current at 47711d3c: 13 files, 159 symbols
stored at C:\Users\you\AppData\Local\lacodda\nooma\data\index\a1b2c3d4e5f60718293a4b5c6d7e8f90.json
```

With `--json`:

```json
{
  "root": "C:\\Projects\\sample",
  "commit": "47711d3c9e...",
  "files": 13,
  "symbols": 159,
  "rebuilt": true,
  "stored_at": "C:\\Users\\you\\AppData\\Local\\lacodda\\nooma\\data\\index\\a1b2c3d4e5f60718293a4b5c6d7e8f90.json"
}
```

| Field | Meaning |
| --- | --- |
| `root` | The work tree the index describes. |
| `commit` | The commit it is pinned to, as full hex. |
| `files` | How many source files were indexed. |
| `symbols` | How many symbols they declare in total. |
| `rebuilt` | `false` when the stored index already matched the checked-out commit and was returned unchanged rather than reparsed. |
| `stored_at` | The file the index was written to. |

## `repo symbols`

```
nooma repo symbols [PATH] [--json] [--store <DIR>] [--name <TEXT>] [--kind <KIND>] [--under <PREFIX>]
```

Lists the symbols the stored index holds, filtered. Fails if the repository has not been indexed, or if the stored index is for a different commit than the one checked out - run `repo index` first.

| Flag | Value name | Filters to |
| --- | --- | --- |
| `--name <TEXT>` | `TEXT` | Symbols whose name contains this text, compared without case. |
| `--kind <KIND>` | `KIND` | Symbols of exactly this kind: one of `function`, `type`, `module`, `constant`. |
| `--under <PREFIX>` | `PREFIX` | Symbols in files whose path starts with this prefix. |

`--under`, not `--path`: the repository itself is the positional `PATH` argument, and two flags sharing a value name would parse as the same flag given twice - clap refuses the whole command rather than guess which one was meant.

```console
$ nooma repo symbols --name ledger --kind type
src/ledger.rs:12  type  Ledger
1 symbols

$ nooma repo symbols --under src/ledger
src/ledger.rs:12  type  Ledger
src/ledger.rs:34  function  Ledger::balance
2 symbols
```

A symbol declared inside another one - a method inside its type - prints qualified as `Parent::name`.

With `--json`, one object per matching symbol:

```json
{
  "commit": "47711d3c9e...",
  "symbols": [
    { "path": "src/ledger.rs", "language": "rust", "name": "Ledger", "kind": "type", "line": 12, "parent": null },
    { "path": "src/ledger.rs", "language": "rust", "name": "balance", "kind": "function", "line": 34, "parent": "Ledger" }
  ]
}
```

Field | Meaning
--- | ---
`path` | The file's path, relative to the repository root, `/`-separated.
`language` | `rust`, `typescript`, `python` or `go`.
`name` | The symbol's name as declared.
`kind` | `function`, `type`, `module` or `constant`.
`line` | The 1-based line the declaration starts on.
`parent` | The enclosing symbol's name, or `null` at file scope.

## `repo deps`

```
nooma repo deps [PATH] [--json] [--store <DIR>]
```

Shows the module dependency graph: which indexed file imports which other indexed file. Only imports that resolve to a file inside this repository become edges - an import of `std::fmt` or an external package has no file here to point at, and is left out of the graph rather than guessed.

```console
$ nooma repo deps
src/ledger.rs
  -> src/entry.rs
src/main.rs
  -> src/ledger.rs
2 files with resolved imports
```

With `--json`:

```json
{
  "commit": "47711d3c9e...",
  "dependencies": {
    "src/ledger.rs": ["src/entry.rs"],
    "src/main.rs": ["src/ledger.rs"]
  }
}
```

`dependencies` maps a file path to the sorted, deduplicated list of indexed files it imports.

## `repo status`

```
nooma repo status [PATH] [--json] [--store <DIR>]
```

Says whether the stored index describes the commit currently checked out, without touching it. The question `rigger` asks before deciding whether to trust what it holds.

```console
$ nooma repo status
current at 47711d3c: 13 files, 159 symbols

$ nooma repo status
stale: indexed at 47711d3c, checked out 9c2e8a1f — run `nooma repo index`

$ nooma repo status
not indexed — run `nooma repo index`
```

With `--json`:

```json
{
  "root": "C:\\Projects\\sample",
  "commit": "9c2e8a1f...",
  "indexed": true,
  "current": false,
  "indexed_commit": "47711d3c9e...",
  "files": 13,
  "symbols": 159
}
```

| Field | Meaning |
| --- | --- |
| `root` | Absolute path of the repository's work tree. |
| `commit` | The commit currently checked out. |
| `indexed` | Whether a stored index exists at all, for any commit. |
| `current` | Whether the stored index matches the checked-out commit. |
| `indexed_commit` | The commit the stored index describes, or `null` if there is none. |
| `files` | How many files the stored index holds, or `null` if there is none. |
| `symbols` | How many symbols the stored index holds, or `null` if there is none. |

A stale on-disk format - an index written by an older `nooma` - is reported the same way as "not indexed": both mean the same instruction, run `repo index`, so `status` does not make the caller tell the two apart.

## Related

- [The repository index](/nooma/concepts/repository-index/) - why the index is pinned to a commit, what a symbol kind means, and what `.nooma-ignore` excludes.
- [Getting Started](/nooma/getting-started/) - the same commands, walked through end to end.
