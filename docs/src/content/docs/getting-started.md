---
title: Getting Started
description: Install nooma and index a repository.
---

## Install

Via cargo:

```bash
cargo install nooma
```

nooma is early: v0.1.0 has no release archive and no install script yet. Building from source with `cargo install` is the only way in for now.

## What works today

v0.1.0 indexes code, not documents. There is no search over files of any kind yet - what exists is `nooma repo`, the repository index that [`rigger`](https://github.com/lacodda/rigger) reads. See [Status](/nooma/#status) for what arrives when.

## Index a repository

Point nooma at a git repository. Any path inside it will do.

```console
$ nooma repo index
indexed 47711d3c: 13 files, 159 symbols
stored at C:\Users\you\AppData\Local\lacodda\nooma\data\index\a1b2c3d4e5f60718293a4b5c6d7e8f90.json
```

Running it again without changes says so instead of doing the work twice:

```console
$ nooma repo index
already current at 47711d3c: 13 files, 159 symbols
stored at C:\Users\you\AppData\Local\lacodda\nooma\data\index\a1b2c3d4e5f60718293a4b5c6d7e8f90.json
```

## Check whether the index is current

```console
$ nooma repo status
current at 47711d3c: 13 files, 159 symbols
```

After a commit, the stored index says so rather than answering with the previous commit's truth:

```console
$ nooma repo status
stale: indexed at 47711d3c, checked out 9c2e8a1f — run `nooma repo index`
```

## List symbols

```console
$ nooma repo symbols --name ledger --kind type
src/ledger.rs:12  type  Ledger
1 symbols
```

`--under` filters by path prefix - the repository path itself is the positional argument, so the filter needed a different name:

```console
$ nooma repo symbols --under src/ledger
src/ledger.rs:12  type  Ledger
src/ledger.rs:34  function  Ledger::balance
2 symbols
```

With `--json`, one line of machine-readable output:

```console
$ nooma repo symbols --name Ledger --json
{"commit":"47711d3c...","symbols":[{"path":"src/ledger.rs","language":"rust","name":"Ledger","kind":"type","line":12,"parent":null}]}
```

## See which files import which

```console
$ nooma repo deps
src/ledger.rs
  -> src/entry.rs
src/main.rs
  -> src/ledger.rs
2 files with resolved imports
```

Only imports that resolve to another file in this repository become edges here. An import of `std::fmt` or a package from outside the repository stays a fact on the file's own record, not a graph edge - see [why imports resolve the way they do](/nooma/concepts/repository-index/).

## Next steps

- Read the [repository index](/nooma/concepts/repository-index/) concept: why the index is pinned to a commit, what a symbol kind means, and what `.nooma-ignore` is for.
- See the full [`nooma repo` reference](/nooma/reference/repo/) for every subcommand and flag.
