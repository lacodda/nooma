---
title: find, similar, index, source
description: Search your folders by the words in them and by what they mean - sources, the index, the vectors, a query and a passage.
---

Four commands make up document search: `nooma source` says which folders to search, `nooma index` reads them, `nooma find` answers a query and `nooma similar` finds what a passage is like. The window, `nooma-app`, is the same library behind one search field.

nooma searches in two halves. The **exact** half finds a document because the words of the query occur in it. The half **by meaning** finds it because it is about the same thing, whatever its words - once the [model](/nooma/reference/model/) is on the machine and `nooma index` has read the library with it; how it works is on [Search by meaning](/nooma/concepts/meaning/). `find` answers with both lists, side by side. They become one ranked list in a later version; see [Status](/nooma/#status).

## Arguments every command takes

| Argument | Meaning |
| --- | --- |
| `--store <DIR>` | Keep the library in this directory instead of the user's data directory. |
| `--models <DIR>` | Look for the model in this directory instead of the user's data directory; `index`, `find` and `similar` take it. Also read from `NOOMA_MODELS`. |
| `--json` | Print one JSON document instead of a report, where the command has one. |

What `--json` prints is a contract: fields are added, never renamed or dropped without a format bump.

## `source`

```
nooma source add <FOLDER> [--exclude <PATTERN>]...
nooma source remove <FOLDER>
nooma source list [--json]
```

A **source** is a folder. Everything under it that is a text document - `.md`, `.markdown`, `.txt`, `.text` - is searched, except what is left out:

- hidden files and folders, such as `.obsidian/`, `.trash/` and `.git/`;
- whatever a `.gitignore` inside a git repository excludes;
- whatever a `.nooma-ignore` file excludes - the same syntax as `.gitignore`, anywhere in the folder;
- whatever `--exclude` patterns the source was added with, in the same syntax, relative to the folder;
- files larger than 16 MB.

Two sources may not overlap. A folder inside a source would put its files in the index twice, and a folder around one would make the inner one's exclusions mean nothing; either is refused, naming the source it overlaps.

Adding or removing a source changes nothing in the index until the next `nooma index`, `nooma find` or `nooma similar`.

`source list --json` prints:

| Field | Meaning |
| --- | --- |
| `sources` | The folders, each with `path` and, if it has any, `exclude`. |
| `documents` | Documents in the index. |
| `chunks` | Chunks in the index - see below. |
| `indexed_at` | When the last update finished, in seconds since the epoch; `null` before the first. |
| `stale` | Why the stored index has to be rebuilt before it can be searched, or `null`. |

## `index`

```
nooma index [--json]
```

Brings the index up to date with the folders. It reads only what changed: a file whose size and modification time are as they were is not opened, and one whose bytes hash as before is not parsed again. Running it twice in a row reads nothing the second time.

A source whose folder cannot be reached - a drive that is not plugged in - keeps its documents in the index as they were, and the report names it. Its files did not go anywhere, and reindexing all of them on every reconnect would be the cost of pretending they had.

A file that is not UTF-8, or UTF-16 with a byte order mark, is skipped and named with the reason. Its encoding is not guessed: a wrong guess puts text in the index that no query will ever match, and says nothing.

When the stored index was built by a nooma that cut or stemmed documents differently, it is thrown away and built again, and the report says why. Searching an index built by other rules would quietly find less.

Only one process updates a library at a time. A second one fails with a message saying so; `nooma find` in that case answers from the stored index instead.

**Then the vectors.** When the model is on the machine, `index` goes on to compute a vector for every passage that has none yet, and to bring the vector index up to date - see [Search by meaning](/nooma/concepts/meaning/). The first time this is the long part: a model on four threads reads three to eight passages a second, so a few thousand notes take an hour or two. A line on stderr counts the passages and, once there is a pace to tell it by, the time left. Stopping it with `Ctrl+C` loses nothing: every vector is kept as it is computed, and the next `index` carries on where this one stopped. Later runs compute only what changed, usually in seconds.

Without the model, `index` updates the exact half and says how to fetch the model. When another nooma - the window, most likely - is computing the vectors of the same library, `index` leaves them to it and says so.

`index --json` prints:

| Field | Meaning |
| --- | --- |
| `documents` | Documents in the index after the update. |
| `indexed` | Documents read and indexed in this update. |
| `removed` | Documents that left the index: deleted, excluded, or in a removed source. |
| `chunks` | Chunks written in this update. |
| `skipped` | Files found and not read, each with `path` and `reason`. |
| `unavailable` | Sources whose folder could not be reached. |
| `rebuilt` | Why the whole index was built again, or `null`. |
| `took_ms` | How long the update took. |
| `vectors` | What computing the vectors did, or `null` when there is no model or another nooma is computing them. |

`vectors` carries:

| Field | Meaning |
| --- | --- |
| `model` | The model the vectors are computed with. |
| `chunks` | Chunks in the library. |
| `passages` | Distinct passages among them; identical chunks share a vector. |
| `embedded` | Passages the model read in this update. |
| `dropped` | Vectors of passages no chunk has any more, dropped from the store. |
| `rebuilt` | Why the store of vectors was started again, or `null`. |
| `took_ms` | How long the update of the vectors took. |
| `embed_ms` | How much of it the model spent reading. |

## `find`

```
nooma find <QUERY> [--limit <N>] [--json] [--no-refresh]
```

Prints two lists, each best first and one entry per document, at the place in it that matched best: the documents that contain the words of the query, then the documents closest to it in meaning. `--limit` caps each list.

The exact list is described first.

**Any form of a word finds the others.** Every word is reduced to its stem before it is stored and before it is looked up, in Russian or in English by the alphabet it is written in: `договоров` finds *Договор*, `warranties` finds *warranty*. A word is judged on its own rather than by the language of the document around it, so the English terms in a Russian note are found by their English forms. `ё` and `е` are the same letter. Numbers and codes are kept as written, and `INV-2025-0114` finds that invoice number as a sequence rather than any document containing `2025`.

**Every word must occur** - in the text, the title, a heading, the tags or the links - as long as some document has them all. When none does, documents with any of the words are returned instead, ranked by how many they hold and how rare those words are.

**Markdown is read as a vault.** A heading ends one chunk and names the next; a code block is never split. From an Obsidian vault, frontmatter `tags`, `aliases` and `title`, inline `#tags` and `[[wikilinks]]` are fields of their own, so a note is found by what it is tagged with. A note is also found - more weakly - by the titles of the notes that link to it, and a note that many others link to ranks slightly higher. A link resolves within its own source, by file name or by the end of its path; a name two notes share resolves to neither.

**The list by meaning** comes from the vector index. Its score is the cosine between the question and the chunk - 1 the same direction, 0 unrelated - and unlike a full-text score it means the same from one search to the next. Its fragment is the opening of the chunk: nothing is highlighted, because no word of the question has to occur in it. When there is no model, or no vectors yet, the list is left out and a line says how to get it; when the vectors cover only part of the library - files changed since the last `nooma index` - the line says how many documents they do not cover yet. A document deleted since is never in it.

`find` brings the full-text index up to date first, like `nooma index`, because answering from a stale index costs a wrong answer and the update costs a look at each file's size and time. It does not compute vectors: that is indexing, and can take an hour, so it is `nooma index` that does it. `--no-refresh` answers from what is stored.

`find --json` prints:

| Field | Meaning |
| --- | --- |
| `query` | The query, as given. |
| `refreshed` | Whether the index was brought up to date before answering. |
| `indexed_at` | When the index was last updated, in seconds since the epoch. |
| `hits` | The documents that contain the words, best first. |
| `meaning` | The documents closest in meaning, with what the vector index could say. |

`meaning` carries:

| Field | Meaning |
| --- | --- |
| `model` | The model that answers. |
| `state` | `ready`; `no-model` when the model is not on the machine; `no-vectors` when it is and nothing has been computed with it yet. |
| `behind` | Documents the vector index does not cover yet. |
| `hits` | The documents, closest first; empty unless `state` is `ready`. |

Each hit, in either list, carries:

| Field | Meaning |
| --- | --- |
| `path` | The file. |
| `kind` | `markdown` or `text`. |
| `title` | The frontmatter title, the first top-level heading, or the file name. |
| `headings` | The headings above the matching chunk, outermost first. |
| `line` | The line the matching chunk starts on. |
| `fragment` | A piece of the matching chunk, around the words that matched. |
| `highlights` | Where the matching words are in `fragment`, as `[start, end]` byte offsets; empty for a hit by meaning. |
| `tags` | The document's tags. |
| `score` | How well it matched. For an exact hit, comparable within one search, not across two; for a hit by meaning, the cosine. |

## `similar`

```
nooma similar <FILE> [--limit <N>] [--json] [--no-refresh]
nooma similar - < passage.txt
```

Finds the documents that say what a passage says: a paragraph from a letter, a page of a note, anything you have in front of you and want the rest of your files' take on. `-` reads the passage from standard input. When the file is itself in the library, it is left out of its own results.

Only the half by meaning answers: a paragraph's words are not a query - every one of them would have to occur. The passage is read as a passage, the way the documents were, rather than as a question; the model reads its first 512 tokens, about a page. `similar` needs the model and the vectors, and says which is missing.

`similar --json` prints the same object as `meaning` in `find --json` - `model`, `state`, `behind` and `hits` - and `example`, the file as given, or `-`.

## Chunks

A document is indexed as **chunks**: runs of paragraphs under one heading, packed up to 1,500 characters. A heading always starts a new chunk, a paragraph is split only when it is longer than a chunk on its own, and a fenced code block stays whole. A result points at its best chunk, so the fragment and the line are where the answer is rather than the top of the file. The half by meaning reads the same chunks, each with its document's title and the headings above it.
