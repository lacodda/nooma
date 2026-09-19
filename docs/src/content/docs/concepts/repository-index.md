---
title: The repository index
description: What a revision is, how only changed files are reparsed, what a symbol kind means, and what the two version numbers protect against.
---

`nooma-core` answers one question: what is in this repository, right now? The answer is a list of files, and each file holds its symbols and its imports. Nothing here is a tree - every consumer so far wants either "all symbols named X" or "everything in this file", and a tree makes both slower to answer than a scan.

## A revision, not just a commit

The index is not "the index of this repository" - it is the index of this repository *at one revision*. A revision is two things: the commit that was checked out, and whether the working tree still matched it.

A commit alone is not enough, and the reason is ordinary: most of the time a developer is looking at a tree that has edits in it. An index that recorded only the commit would answer `current` for "the tree at this commit plus three unsaved files", which tells the caller their own newest work is already indexed. Recording both makes that answer unrepresentable rather than something a caller has to remember to check. A revision with edits prints as `29028610+dirty`.

`dirty` is decided by comparing content hashes against what the commit holds - not by asking `git status`. The two disagree exactly where it matters: a file written since git last looked is changed by every measure that counts here and by none that git reports yet. Trusting git there would leave the newest edit out of the index.

Both directions are compared. A file the tree has and the commit does not is a difference; so is a file the commit has and the tree has lost. The second one is invisible to any question asked about the working tree, since a walk only ever reports files that exist - so the commit is asked for its own list and the two are compared as sets.

## Only what changed is parsed

Every file carries the BLAKE3 hash of its bytes. A second pass reads and hashes everything, and sends only the mismatches to the parser.

That split is deliberate: reading and hashing a megabyte costs milliseconds, and parsing a repository of a few hundred files costs seconds. On a 287-file repository the difference is 4.5 seconds against 165 milliseconds. So there is no attempt to avoid reading - only to avoid parsing, which is where the time actually goes.

Git's own diff is not consulted, for the same reason `git status` is not: it would be a second source of truth about what changed, and it is wrong about the file someone is editing right now. One mechanism covers both questions the feature was asked - what changed between two commits, and what changed in the working tree - because both reduce to the same comparison.

Two properties make the shortcut safe to rely on:

- **Reuse gives exactly what parsing gives.** An entry carried across is the entry a full parse would have produced; `--force` parses everything and must produce an identical index. A fast path that can differ from the slow one is a way of being wrong quickly.
- **The result does not depend on what was skipped.** Files are sorted by path before the index is written, so an index built by reusing 214 entries and parsing one is byte-for-byte the index built by parsing all 215.

Because reparsing costs only what changed, `repo symbols` and `repo deps` bring the index up to date before answering instead of refusing. v0.1.0 refused, which was right when a full reparse was the only repair available; once the repair became cheap, the refusal was the greater surprise. What the refresh cost is printed on stderr - work done on a caller's behalf is never silent - and `--no-refresh` answers from the stored index for a caller that wants exactly that.

## What gets left out, and why two files decide it

`repo index` walks the work tree, not the commit's tree in git's object database - that is what the developer is actually looking at, and a tree with edits in it is the normal case rather than the exception.

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

## From identifiers to text with meaning in it

A symbol entry answers *where is `RepoIndex` declared*. That is a lookup, and a list of identifiers is the right shape for it. It is the wrong shape for the question this product is actually for — *what is this module about* — because a bare name carries almost no meaning to match against.

So a file also carries a **module summary**: its header comment, and each declaration's signature and documentation. Nothing in it is generated or inferred. A signature is the declaration as written, up to where its body starts; a doc is the comment the author attached to it. A summary that invented a description would be one nobody could trust, and this product's promise is precisely that the machine did not add anything the author did not write.

Bodies are left out. The body of a function is the one part of it every caller is entitled to ignore, and leaving it out is also what keeps a summary small enough to embed whole.

One mechanism serves four languages, because the symbol query already wraps each whole declaration — a property it has for working out what encloses what. Two details have to be right, and both are silent when wrong:

- **A doc comment can sit above a wrapper rather than above the declaration.** `export function f() {}` parses as an export statement around the declaration, and the comment sits above the export. Walking the declaration's own siblings finds nothing — and it finds nothing for exactly the exported declarations, which is the public API a summary exists to describe.
- **Not every comment is documentation.** Rust marks `///` and `/**` in the grammar; a plain `//` is an aside. Without the distinction a `// TODO: rewrite this` becomes a function's description, and the noise goes into the text an embedder reads. Go makes no such distinction and needs none: a comment above a declaration is its documentation, by the language's own convention.

## Commit messages are documents too

A repository's history is already a corpus of short documents about the code, written by the people who changed it, saying *why*. That why is nowhere else — the code says what it does now, and only the commit that introduced it says what it was for.

So the history is read alongside the index: each commit's message, and the paths it touched. *The commit where the PATH handling was fixed* is a question the file index cannot answer at all.

It is cached on the opposite principle from a file. A file is keyed by the hash of its contents, because a file changes under a stable name. A commit's id *is* the hash of everything about it, so a commit already read never needs reading again — there is no comparison pass and nothing to invalidate.

But "already read" answers only whether to read it, never whether it belongs. What belongs is what HEAD reaches, and the two part company under `--amend` and rebase: a rewritten commit is a different object, so the original stays in the stored history. A history that carried the stored list across from wherever a walk stopped would keep that original alive beside its replacement, claiming a commit the repository no longer has. Walking every reachable commit and reusing only what is still reachable costs one pass over a list of hashes and makes that unrepresentable.

## Two version numbers, protecting two different things

The stored index carries two numbers, and each guards against a different way of being quietly wrong.

**`format_version`** covers the file's shape, and is checked before anything else in the file is deserialized. A reader that meets a different number refuses the file outright and says to reindex; it never reads it under today's assumptions. The alternative is the worst failure available to a search tool - a field that changed meaning gets believed under the new meaning, and results stop being right in a way nobody notices. "Stopped finding things" reads as an empty index or a bad query, not as the format having moved out from under it.

**`chunker_version`** covers what an entry *means*. Change a tree-sitter query, a symbol kind, how a file is divided, or what a summary lifts out of it, and every stored entry now says something different - while the file's bytes, and therefore its hash, are exactly as they were. The hash check would happily reuse all of them. Without a number to compare, entries cut up by the old rules would be kept and extended by the new ones, and the index would become a mixture of two vocabularies. A mismatch forces a full reparse; it does not make the file unreadable, which is what keeps it a separate number rather than a bump of the first.

## Related

- [`nooma repo` reference](/nooma/reference/repo/) - the commands that build, read and check this index.
- [ADR 0003](https://github.com/lacodda/nooma/blob/main/docs/adr/0003-repository-index-as-a-library.md) - why the index shipped first, and as a library.
