---
title: The repository index
description: Why the index is pinned to a commit, what a symbol kind means, and what format_version protects against.
---

`nooma-core` answers one question: what is in this repository, at this commit? The answer is a list of files, and each file holds its symbols and its imports. Nothing here is a tree - every consumer so far wants either "all symbols named X" or "everything in this file", and a tree makes both slower to answer than a scan.

## Pinned to a commit hash

The index is not "the index of this repository" - it is the index of this repository *at this commit*. That distinction is the whole design.

A caller holding an answer can ask whether it is still the answer: `repo status` compares the commit stored in the index against the commit currently checked out. Two callers asking about the same commit get the same bytes, because the files are sorted by path before being written - an index built on Windows and one built in CI on Linux describe the same commit identically.

A stale index is refused, not read. `repo symbols` and `repo deps` fail outright if the stored index is for a different commit than the one checked out, rather than answering with the previous commit's truth. That failure mode - a query returning yesterday's symbols with today's confidence - is the kind of quiet wrongness that takes an afternoon to notice, and the index is built specifically to make it loud instead: a refusal that names the commit it has and the commit you're on, and says to reindex.

## What gets left out, and why two files decide it

`repo index` walks the work tree, not the commit's tree in git's object database - that is what the developer is actually looking at, and it is what the next version's incremental pass has to handle anyway, since a dirty tree is the normal case, not the exception.

Two files decide what is walked:

- **`.gitignore`** is honored because git honors it. `target/` and `node_modules/` are build output, not source - indexing them would bury a repository's own symbols under its dependencies', and `Ledger` would compete in a symbol search against every `Ledger`-shaped name any dependency happens to declare.
- **`.nooma-ignore`** sits beside it with the same syntax, for what belongs in git but not in the index: vendored code, generated bindings, fixtures. Content that is legitimately committed but is not a repository's own decisions.

Only files whose extension names a language nooma understands are considered in the first place - by extension alone, never by sniffing content. A repository's files are named by their authors, and second-guessing `.rs` by reading bytes would cost a read of every file in the tree to change the answer approximately never.

## What a symbol kind means, and why there are only four

A symbol is deliberately described coarsely: `function`, `type`, `module`, or `constant`. A union is a type. A trait is a type. A Go interface is a type. Consumers ask "where is `RepoIndex` declared", never "was it a struct or an enum" - splitting the kind finer would make every caller match on four spellings of the same idea, for a distinction nobody downstream has asked to make.

Each symbol also records its line and, when it sits inside another declaration, its parent - a method's type, a nested function's enclosing function. That is enough to answer "what does this file declare" and "where is X" without needing the kind to carry more than it does.

## Imports: stored as written, resolved only when honest

An import is stored exactly as the source wrote it - `std::collections::BTreeMap`, `./widgets/Button`, `os.path`, `net/http` - not resolved to a file at index time. Resolving it needs the language's whole module system: tsconfig path aliases, Python's `sys.path`, Go modules with their own resolution rules. Guessing wrong is worse than not resolving at all, because a wrong edge in a dependency graph looks exactly like a right one until someone trusts it.

The module dependency graph (`repo deps`) resolves only what can be resolved honestly: relative imports, because the source itself says exactly where to look - `./entry` from `src/ledger.rs` can only mean `src/entry.rs` or one of its language-specific candidates (`.ts`, `/index.ts`, `/__init__.py`, and so on). An import like `crate::store` is never resolved this way, because it could be this repository or a dependency of the same name, and there is no honest way to tell without doing what a compiler does. Such imports remain visible on the file's own `imports` list - a fact about the file - but never become an edge in the graph.

## `format_version`: what it protects against

The stored index carries a `format_version` field, checked before anything else in the file is even deserialized. A reader that meets a different number refuses the file outright and says to reindex - it never attempts to read it under the wrong assumptions.

This exists because the alternative failure mode is the worst one available to a search tool: a field that changed meaning between versions gets deserialized under the new meaning and quietly believed, and search results stop being wrong in a way anyone would notice. "Stopped finding things" reads as the index being empty or the query being bad, not as the format having moved out from under it. Refusing loudly, with an instruction to reindex, converts that into a one-line fix instead of a debugging session.

## Related

- [`nooma repo` reference](/nooma/reference/repo/) - the commands that build, read and check this index.
- [ADR 0003](https://github.com/lacodda/nooma/blob/main/docs/adr/0003-repository-index-as-a-library.md) - why the index shipped first, and as a library.
