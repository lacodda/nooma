# Changelog

All notable changes to this project are documented in this file.

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
