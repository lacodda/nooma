# Changelog

All notable changes to this project are documented in this file.

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
