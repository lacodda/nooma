# Changelog

All notable changes to this project are documented in this file.

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
