# Changelog

All notable changes to this project are documented in this file.

## [0.11.0] - 2026-10-21

### Bug Fixes
- Link clang's runtime for the model on macOS

### Build
- Update the stack

### Documentation
- Describe Word documents, books and encodings

### Features
- Read Word documents, books and text in any encoding

### Breaking Changes

the full-text index goes to format 4 and is built
again on the next update, vectors kept. The new rules stem some
English words otherwise - added is add, no longer ad.

in nooma-core DocKind has Docx and Epub and no
longer has of(), since a name no longer decides a kind - reading
does; UpdateReport carries guessed. The JSON of index prints guessed,
and a hit's kind may be docx or epub. Text that is not UTF-8 is read
rather than skipped.

## [0.10.0] - 2026-10-16

### Documentation
- Describe PDF search

### Features
- Find a PDF at its page

### Breaking Changes

the full-text index goes to format 3 and is built
again on the next update. In nooma-core the Chunk and Hit structs
carry a page, DocKind has Pdf and a size limit of its own in place
of the MAX_FILE_BYTES constant, and UpdateReport carries textless.
The JSON of find and similar prints page for a PDF, and the JSON of
index prints textless.

## [0.9.0] - 2026-10-12

### Build
- Update the stack

### Documentation
- Describe search across languages

### Features
- Find a document in the other language

### Breaking Changes

the vector index goes to format 2 and is rebuilt
from the stored vectors on the next update, with nothing read again.
In nooma-core Model::language is required, hybrid::similar takes the
passage text, Hit carries closest and Ranked carries across; the JSON
of find and similar prints both.

## [0.8.0] - 2026-10-08

### Bug Fixes
- Let a waiting search have the model on every system

### Documentation
- Describe one list from two halves

### Features
- Rank the words and the meaning as one list
- Show one list from both halves

### Testing
- Watch every stored format since the last tag

### Breaking Changes

the full-text index goes to format 2 - every chunk
keeps its file's modification time and every searched field - and is
rebuilt on the next update, in seconds; the vectors are kept. In
nooma-core the full-text search returns Matched, with the share of the
query each hit holds and all_words, and every Hit carries modified.
The JSON of find prints one hits list of ranked documents, each with
the place the words and the meaning gave it, plus all_words; meaning
no longer carries hits. The JSON of similar prints the same shape.

## [0.7.0] - 2026-10-05

### Documentation
- Describe the vector index, the two lists and search by example

### Features
- Search by meaning through a graph index over the vectors
- Search by meaning beside the words, read in the background

### Breaking Changes

in nooma-core, the vector store and its index are
behind the `semantic` feature; `Library::semantic` takes a `Model`
and returns `Option<SemanticIndex>` that answers over what it covers
instead of failing; `Embedder` is split into `Model` and `Embedder`,
and `Error::VectorsBehind` counts documents. `nooma find --json` adds
`meaning`, and `nooma index --json` adds `vectors`.

## [0.6.0] - 2026-09-26

### Bug Fixes
- Keep a Rust doc above its attributes, and test-only items off the surface

### CI
- Fetch the model once and lint the core without its runner

### Documentation
- Describe search by meaning and how the model was chosen

### Features
- Keep what nooma builds in the machine's local data directory
- Pin the embedding models, fetch them in one crate, keep vectors per model
- Add `nooma model` and `nooma eval`

### Breaking Changes

the repository index chunker is now 3. The first read
after upgrading reparses every file to rebuild its summary; there is
nothing to do by hand.

on Windows the library and the repository indexes move
from %APPDATA%\lacodda\nooma\data to %LOCALAPPDATA%\lacodda\nooma\data.
Nothing is carried over: add your sources again with `nooma source add`,
or move the `library` folder across before the first run. Repository
indexes rebuild on their own at the next `nooma repo` command. Linux and
macOS are unaffected.

## [0.5.0] - 2026-09-23

### Features
- Search folders of text through a stemmed full-text index
- Search your folders from a window

### Performance
- Keep the index open between searches

### Documentation
- Say how to check a default build, with a check that works
- Describe document search, the window and the build

### CI
- Build the window and ship it as installers

## [0.4.0] - 2026-09-20

### Features
- Describe a module in prose with your own Claude Code

### Testing
- Fail when a stored version bump ships unmarked

## [0.3.0] - 2026-09-19

### Features
- Summarize a module and read commit messages as documents
- Add `repo summary` and `repo history`

### Bug Fixes
- Keep tool directives and comment margins out of a summary

### Miscellaneous Tasks
- Update the stack before v0.3.0

### Breaking Changes

the chunker version is 2, so the first `nooma repo index` after
upgrading reparses every file. Nothing has to be deleted and no command
changes: a file's bytes do not move when the rules for reading it do, so
without the bump an index built before summaries existed would be reused
and every file in it would answer "no summary" forever.

`FileIndex` gains `summary`, which is `None` only for a file the parser
could not read. Commit history is stored in its own file beside the
index, under its own format number.

## [0.2.0] - 2026-09-18

### Documentation
- Describe revisions and the incremental pass

### Features
- Reparse only the files whose contents changed

### Breaking Changes

the stored index format is 2; a format 1 index is
refused and rebuilt on the next `nooma repo index`. `RepoIndex::commit`
is now `RepoIndex::revision.commit`, and `--json` gains `dirty` on every
subcommand plus the per-pass counts on `index`.

## [0.1.0] - 2026-09-18

### Bug Fixes
- Declare the MSRV that actually builds

### CI
- Build the workspace, and make a tag produce a release

### Documentation
- Make the readme a shopfront
- Drop the duplicate heading, add the badges
- Put up the docs site and document `nooma repo`
- Add the changelog for v0.1.0

### Features
- Scaffold the project
- Add the lacodda line mark and derived assets
- Index a repository into symbols, imports and dependencies
