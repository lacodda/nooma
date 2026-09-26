---
title: model
description: The embedding models - which there are, which are on this machine, fetching one and removing one.
---

Search by meaning needs an embedding model: a network that turns a passage of text into a vector, so that passages about the same thing land near each other. nooma runs it on your CPU. `nooma model` is where it comes from.

```
nooma model list [--json]
nooma model fetch [<MODEL>]
nooma model remove <MODEL>
```

**`nooma model fetch` is the one thing nooma does over the network.** It downloads the model's files and nothing else, sends nothing about your files, and only runs when you run it. Every other command that needs a model reads it from disk; when it is not there, the command says so and names this one. It never downloads on its own.

## Arguments

| Argument | Meaning |
| --- | --- |
| `--models <DIR>` | Keep models in this directory instead of nooma's data directory. Also read from `NOOMA_MODELS`. |
| `--store <DIR>` | The library whose vectors `remove` deletes, as for [`find`](/nooma/reference/find/). |
| `--json` | Print one JSON document instead of a report (`list`). |

Models live in nooma's local data directory, beside the library:

| System | Folder |
| --- | --- |
| Windows | `%LOCALAPPDATA%\lacodda\nooma\data\models` |
| macOS | `~/Library/Application Support/com.lacodda.nooma/models` |
| Linux | `~/.local/share/nooma/models` |

The window and the command line share them: a model fetched once serves both.

## The models

| Model | Vectors | Size | License |
| --- | --- | --- | --- |
| `multilingual-e5-small` - **the one nooma uses** | 384 | 487 MB | MIT |
| `paraphrase-multilingual-minilm-l12-v2` | 384 | 487 MB | Apache-2.0 |
| `bge-m3` | 1024 | 2.3 GB | MIT |

All three read Russian and English; nooma is pointed at archives where both are mixed in one note. The first is the one nooma uses - it was chosen by measuring all three on the same questions, and the measurement, including what it is weak at, is on [Search by meaning](/nooma/concepts/meaning/#how-the-model-was-chosen). The other two stay in the list so the measurement can be repeated on your own folders with [`nooma eval`](/nooma/reference/eval/).

A model is not a name on the Hub - a repository there can be changed by a new commit at any time, and a vector from one set of weights means nothing next to a vector from another. So each model is pinned: the commit of its repository, and the size and SHA-256 of every file taken from it. What was measured is what is fetched.

## `list`

Every model nooma can run, its size, and whether it is on this machine.

```
$ nooma model list
multilingual-e5-small                   384 dims   487 MB  on this machine  · used by nooma
paraphrase-multilingual-minilm-l12-v2   384 dims   487 MB  not fetched
bge-m3                                 1024 dims   2.3 GB  not fetched

models live in C:\Users\you\AppData\Local\lacodda\nooma\data\models
```

`list --json` prints:

| Field | Meaning |
| --- | --- |
| `dir` | The folder models are kept in. |
| `models` | One entry per model, with the fields below. |
| `id` | The name nooma calls it by, and the name of its folder. |
| `repository` | The repository on the Hugging Face Hub its files come from. |
| `revision` | The commit of that repository they were taken at. |
| `license` | The license the weights are published under. |
| `dimensions` | The length of its vectors. |
| `bytes` | Its size on disk. |
| `present` | Whether every file is here, at the size it was pinned at. |
| `default` | Whether it is the model nooma uses. |

## `fetch`

Downloads a model from the Hub - the one nooma uses when none is named.

- Each file is written beside its place as `<name>.part`, checked against the size and SHA-256 it was pinned with, and only then renamed into place. A models folder never holds a file that is not the one measured.
- An interrupted fetch resumes: run it again and a `.part` carries on from where it stopped. A connection that goes quiet is replaced, and the rest asked for on a new one.
- A file that arrives different from its pin is deleted and the fetch fails, saying so. Run it again; if it happens again, the file upstream has changed and this version of nooma cannot use it.
- Files already in place are hashed and kept when they match, so `fetch` on a fetched model checks it and downloads nothing.

## `remove`

Deletes a model's files, and the vectors computed with it in the library. Nothing else uses them: vectors from one model are meaningless to another, which is why each model keeps its own.
