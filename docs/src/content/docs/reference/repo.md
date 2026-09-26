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
stored at C:\Users\you\AppData\Local\lacodda\nooma\data\index\5d50076687f7bd4e.json

$ nooma repo index
29028610: 3 files, 4 symbols
nothing changed; reused 3 files
stored at C:\Users\you\AppData\Local\lacodda\nooma\data\index\5d50076687f7bd4e.json
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
  "stored_at": "C:\\Users\\you\\AppData\\Local\\lacodda\\nooma\\data\\index\\5d50076687f7bd4e.json"
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

## `repo summary`

```
nooma repo summary [PATH] [--json] [--under <PREFIX>] [--all] [--no-refresh] [--store <DIR>] [--prose]
```

Describes what each module offers: its header comment, and the signature and documentation of each declaration it makes visible. Every line of it is text the author wrote — a signature is the declaration up to its body, a doc is the comment attached to it, and nothing is generated or inferred.

Private declarations are left out unless `--all` is given. What a module keeps to itself is not part of what it offers, and a surface that listed it would describe something other than the module's contract.

```console
$ nooma repo summary --under src/ledger.rs
src/ledger.rs
  Keeps entries in balance.
      4  pub fn post(entry: Entry) -> Result<Balance>
     18  pub struct Entry
```

| Option | What it does |
| --- | --- |
| `--under <PREFIX>` | Only modules whose path starts with this. |
| `--all` | Include declarations the language keeps private. |
| `--no-refresh` | Answer from the stored index without bringing it up to date. |
| `--prose` | Add a generated paragraph saying what each module is for. Needs a build made with the `prose` feature, and the Claude Code CLI — see [prose summaries](/reference/prose/). |

With `--json`:

```json
{
  "commit": "29028610740cc923baf966791b1e2090597fa546",
  "dirty": false,
  "modules": [
    {
      "path": "src/ledger.rs",
      "language": "rust",
      "header": "Keeps entries in balance.",
      "public": 2,
      "text": "src/ledger.rs

Keeps entries in balance.

pub fn post(entry: Entry) -> Result<Balance>
Post an entry to the ledger.",
      "entries": [
        {
          "name": "post",
          "kind": "function",
          "line": 4,
          "parent": null,
          "signature": "pub fn post(entry: Entry) -> Result<Balance>",
          "doc": "Post an entry to the ledger.",
          "public": true
        }
      ]
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `commit` | The commit the index describes. |
| `dirty` | Whether the tree had uncommitted edits when it was indexed. |
| `modules` | One entry per module, in path order. |
| `path` | The module's path, relative to the repository root. |
| `language` | The language it was read as. |
| `header` | The module's own comment, or `null`. |
| `public` | How many declarations the module makes visible outside. |
| `text` | The whole summary as one block: the path, the header, then each public signature with its doc. |
| `entries` | The declarations, in source order. |
| `name` | What it is called. |
| `kind` | `function`, `type`, `module` or `constant`. |
| `line` | The 1-based line it is declared on. |
| `parent` | The enclosing declaration, or `null`. |
| `signature` | The declaration as written, up to its body, with whitespace collapsed. |
| `doc` | The documentation attached to it, markers stripped, or `null`. |

`text` is the field to hand to something that reads prose. The rest is for a caller that wants the parts separately.

Everything above is lifted from the source word for word. Nothing in this command is generated unless [`--prose`](/reference/prose/) asks for it, and what it adds arrives in a field of its own.

### What counts as visible

Each language says it differently, and all four say it somehow.

| Language | Public means |
| --- | --- |
| Rust | A bare `pub`. `pub(crate)`, `pub(super)` and `pub(in ...)` are visible inside the crate and are not part of its surface. |
| TypeScript | `export`, including a declaration inside an exported class. |
| Go | An initial capital. |
| Python | A name that does not begin with `_`. |

## `repo history`

```
nooma repo history [PATH] [--json] [--matching <TEXT>] [--touching <PATH>] [--limit <N>] [--no-refresh] [--store <DIR>]
```

Reads the commit messages as documents. The code says what it does now; only the commit that introduced it says what it was for, and that reason is in the message and nowhere else — which is what makes *the commit where the PATH handling was fixed* a question this answers and the file index cannot.

```console
$ nooma repo history --matching "path handling"
4fa18bad  fix: correct the PATH handling on Windows

$ nooma repo history --touching src/ledger.rs
29028610  feat: add double entry
1c0ffee0  feat: add the ledger
```

Naming both narrows by both.

| Option | What it does |
| --- | --- |
| `--matching <TEXT>` | Only commits whose message contains this, compared without case. Summary and body are both searched. |
| `--touching <PATH>` | Only commits that changed this path, or anything beneath it. |
| `--limit <N>` | How many commits to read from the top. Defaults to 5000. |
| `--no-refresh` | Answer from the stored history without reading new commits. |

With `--json`:

```json
{
  "head": "29028610740cc923baf966791b1e2090597fa546",
  "commits": [
    {
      "id": "29028610740cc923baf966791b1e2090597fa546",
      "summary": "feat: add double entry",
      "body": "Lines sum to zero in every currency.",
      "author": "A Writer",
      "time": 1789856815,
      "paths": ["src/ledger.rs"]
    }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `head` | The commit the history was read from. |
| `summary` | The first line of the message. |
| `commits` | The commits, newest first. |
| `id` | The full hex object id. |
| `body` | The rest of the message below the summary, or `null`. |
| `author` | Who wrote the change. |
| `time` | When it was written, in seconds since the epoch, UTC. |
| `paths` | The paths the commit changed, against its first parent. |

A merge is compared against its first parent only. Comparing against all of them reports every path either side touched, which makes a merge look like the work it merged and buries the commit that did it.

### How a history is brought up to date

A commit's id is the hash of everything about it, so a commit already read never needs reading again — there is no comparison pass and nothing to invalidate.

What the walk decides is *membership*, not only freshness. The commits reachable from HEAD are the history; the stored one only says which of them have been read. That distinction is what `git commit --amend` and rebase turn on: a rewritten commit is a different object, so the original stays in the stored history, and carrying that history across from the point a walk stopped would keep the original alive beside its replacement — claiming a commit the repository no longer has.

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
