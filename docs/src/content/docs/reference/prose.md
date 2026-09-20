---
title: prose summaries
description: An optional generated paragraph saying what a module is for, using your own installed Claude Code.
---

`nooma repo summary` describes a module using only what its author wrote: the header comment, and each declaration's signature and documentation. That is what makes it trustworthy, and it is also its ceiling. A module whose author wrote no documentation summarizes as a list of signatures, and a list of signatures does not say what the module is *for*.

`--prose` adds a paragraph that does — generated, clearly marked as generated, and never in place of anything lifted from the source.

:::caution[This is the one thing in nooma that leaves your machine]
Everything else nooma does happens locally, by construction. A prose summary is produced by sending a module's summary — its path, header and public signatures with their docs — to Anthropic through the Claude Code CLI you installed. Source bodies are never sent; the summary is.

It is off unless you ask for it, in three separate ways. See [ADR 0004](https://github.com/lacodda/nooma/blob/main/docs/adr/0004-prose-summaries-shell-out.md).
:::

## What you need

Prose summaries use the [Claude Code](https://claude.com/claude-code) CLI you already have, under your own subscription. nooma holds no API key and adds no account: it runs `claude` the way you would and reads the answer.

They also need a build that has them. The feature is not in the default build:

```console
$ cargo install nooma --features prose
```

A build without it does not merely decline to use the CLI — it has no such code path at all, and refuses `--prose` as an unknown argument:

```console
$ nooma repo summary --prose
error: unexpected argument '--prose' found
```

That is deliberate. "Nothing leaves your machine" should be something you can check rather than something you take on trust, and in a default build there is nothing in the binary to check.

## Using it

```
nooma repo summary --prose [PATH] [--json] [--under <PREFIX>] [--store <DIR>]
```

```console
$ nooma repo summary --prose --under src/lang.rs
describing with 2.1.278 (Claude Code) (0 modules already in ...\prose.json)
described 1 modules (0 from the cache); $0.0058
src/lang.rs
  Which languages are understood, and how each one is read.
  [generated] This module defines and manages information about the programming
  languages this build understands, including their names, file extensions and
  the queries used to extract symbols and imports. Callers use it to identify a
  source file's language from its path, or to retrieve a language's metadata.
     16  pub enum Language
     29  pub const ALL: &[Language]
```

The line the author wrote and the line the model wrote sit next to each other, and only the label tells them apart — so the label is always there. In `--json`, the paragraph is a field of its own:

```json
{
  "path": "src/lang.rs",
  "header": "Which languages are understood, and how each one is read.",
  "prose": "This module defines and manages information about ...",
  "text": "src/lang.rs\n\nWhich languages are understood ...",
  "entries": []
}
```

`header` and `text` are lifted from the source, word for word, and `prose` is not. A consumer that folded them together would lose the only distinction that matters here, so they are never folded.

The `prose` field is absent — not `null` — when the paragraph was not asked for.

## What it costs, and how often

Every call is charged to your own subscription, so it is worth knowing the shape of the bill.

Measured over the ten modules of `nooma-core`: **$0.2021**, about two cents a module. The input is the module summary, so the price follows how much documentation the author already wrote — a densely documented module costs more than a bare one. Call it **$2 per hundred modules** as a planning figure.

You pay once per distinct file. Prose is cached by the file's content hash, so:

- Running the command again over an unchanged repository costs nothing.
- Editing one file re-describes that file and nothing else.
- The same file vendored into a second repository is not described twice — the key is the content, not the path.

```console
$ nooma repo summary --prose --under src/
described 10 modules (0 from the cache); $0.2021

$ nooma repo summary --prose --under src/
10 modules described from the cache
```

The cache is a file named `prose.json`, beside the index in the directory `--store` names (or the user data directory by default). Deleting it means paying again; nothing else depends on it.

Unlike the index, it survives a format or chunker bump. Rebuilding an index is free and happens routinely; prose is not free, and a version bump that silently burned it would be a good reason never to use the feature twice.

## What it will and will not say

The model is given the summary and nothing else. It never sees a function body, so it is told, in as many words, not to describe what a function does internally, not to claim how complete a list is, and not to explain an identifier beyond what its own name and doc say. Asked about a module too thin to describe, it is told to say what the module declares and stop.

This is worth knowing because the failure is real rather than theoretical. Given a module stripped of its documentation, an earlier wording produced "a comprehensive list of all supported languages" about a constant it knew only the name of. The instructions above are what that run bought.

Treat a paragraph as a good summary of the surface it was shown, not as a statement about the code. Where it matters, the signatures are right there underneath it.

## When it cannot run

If Claude Code is not on the `PATH`, the command says so once and stops, before spending anything:

```console
$ nooma repo summary --prose
nooma: Claude Code is not on the PATH. Install it from https://claude.com/claude-code,
then run this again — or drop --prose for the summary lifted from the source.
```

Anything the CLI itself refuses — not logged in, out of credit — is passed through in its own words, because the CLI knows what to tell you and nooma does not.
