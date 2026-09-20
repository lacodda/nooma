# 0004 — Prose summaries shell out to the user's own Claude Code, behind a feature flag

Date: 2026-09-20
Status: accepted

## Context

A module summary (ADR-less, v0.3.0) is lifted out of the source: the header comment, and each declaration's signature and doc. Nothing in it is generated. That is what makes it trustworthy, and it is also its limit — a module whose author wrote no documentation summarizes as a list of signatures, which says very little about what the module is *for*.

Prose would say it. Producing prose means a language model, and this product's central promise is that no file content leaves the machine (ADR 0002). Three ways to reconcile that:

1. **A local generative model.** Keeps the promise literally. It also means shipping a multi-gigabyte model, and a small local model writing about code it half-understands produces confident nonsense — the failure mode this feature can least afford.
2. **A cloud API with a key in the config.** Cheapest to build, and it makes nooma a product that sends your source code to a third party. ADR 0002 rejected this for embeddings and the reasoning does not change for prose.
3. **Shell out to the Claude Code CLI the user already installed.** The user's own machine, their own subscription, their own session — nooma never holds a key, never sees a token, and adds no account to the product. kilna established this shape for the same reason.

## Decision

Prose summaries are produced by shelling out to the user's installed Claude Code CLI, and the feature is off unless asked for, in three separate ways at once:

- **It is not in the library.** `nooma-core` — the crate `rigger` and `scheda` link — has no code that starts a process or opens a socket, and gains none. The prose module lives in the `nooma` binary.
- **It is behind a Cargo feature.** `prose` is not in the default features, and the released binaries are default builds. A build that does not ask for it contains no such code path at all, and says so: `--prose` is itself defined behind the feature, so such a build refuses it as an *unknown argument* rather than accepting a flag that quietly does nothing. That refusal is the check, and the test suite asserts it on every run.
- **It is behind a flag at run time.** `nooma repo summary --prose` asks for it; nothing else does. There is no setting that turns it on globally, because a setting is how an offline promise becomes an offline default.

Generated prose is stored apart from the summary lifted from source, and is labelled wherever it is shown.

## Consequences

Positive:

- The offline promise stays exactly as strong as ADR 0002 states it, for every user who does not opt in and for every consumer of the library, with nothing to configure.
- No API key, no account, no billing relationship: the cost lands on a subscription the user already chose.
- Prose is cached by content hash and paid for once per distinct file, across every repository on the machine.

Negative:

- The feature works only for users who have Claude Code installed and logged in. Everyone else sees an explanation and the structural summary.
- The output is not reproducible: two runs over the same file can word it differently. The cache makes this rare rather than impossible.
- nooma now depends on the output shape of a program it does not version. Only the fields it needs are read, and unknown fields are ignored.

Rejected alternatives:

- **A bundled local model** — keeps the promise with no opt-in, at the price of a far larger download and prose nobody should trust.
- **An API key in the config** — the premise ADR 0002 exists to refuse.
- **On by default when the CLI happens to be present** — the worst of the options: it would make "nothing leaves your machine" depend on what else the user has installed.
