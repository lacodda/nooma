# 0003 — The repository index ships first, as a library

Date: 2026-09-18
Status: accepted

## Context

nooma was conceived as an application: a window with one field, searching a personal archive. The plan put documents first and code later.

Two things changed that ordering.

`rigger` — the line's record of projects and tasks — needs to know what is in a repository so it can put module summaries into the context packet it hands an assistant. Building a second code index inside `rigger` would mean two parsers, two caches and two answers to "what is in this file", with no mechanism keeping them honest.

And a search product's hardest half is not the window. It is the index: what a chunk is, what invalidates it, what the format promises and how it migrates. Writing that under a GUI means discovering the constraints through the GUI, which is the slowest way to find them.

## Decision

The repository index is the first block of work, and it ships as `nooma-core` — a library with no UI, no window and no network path — with `nooma` the binary as its first caller.

Three consequences follow from "library first" rather than being separate choices:

**The index is on disk from the first version.** Not because v0.1 is slow enough to need a cache, but because persistence determines the shape of everything above it — the incremental pass, the format version that forces a reindex, the contract a caller reads. Adding it later would mean rewriting all three.

**The index is pinned to a commit hash.** A caller holding an answer can ask whether it is still the answer, and two callers asking about the same commit get the same bytes. A stale index is refused, never read: answering with the previous commit's truth is the kind of quiet wrongness that takes an afternoon to notice.

**The CLI prints JSON.** `rigger` can call `nooma repo symbols --json` today, before either product speaks MCP. A CLI with machine output is the line's standard way for one product to ask another a question, and it costs nothing to keep once MCP arrives.

## Consequences

Positive:

- One code index in the line, with one set of rules about what is indexed and what a symbol is.
- The index format, the migration story and the invalidation rules are settled while they are cheap to change — before a GUI, a vector half and a user's stored data depend on them.
- `rigger` gets something useful from nooma several versions before nooma has a window.

Negative:

- The first releases are useful to another program, not to a person. The MVP for a human moves to v0.8.0, where the hybrid arrives.
- A library consumed in-tree by another product constrains what can be broken later, earlier than a standalone application would have been constrained.

Rejected alternatives:

- **Documents first, code later** — the original plan. It leaves `rigger` to build its own index, which is the second-truth problem this decision exists to avoid.
- **A code index inside `rigger`** — no new crate, no new product surface. But then a search product's core competence lives in a task tracker, and nooma still has to write its own when it reaches code.
- **Publishing `nooma-core` to crates.io from v0.1** — a public API frozen before its only two callers have used it in anger. The crate is consumed by path until the surface has stopped moving.
