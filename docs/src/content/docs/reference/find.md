---
title: find, similar, index, source
description: Search your folders by the words in them and by what they mean - sources, the index, the vectors, a query and a passage.
---

Four commands make up document search: `nooma source` says which folders to search, `nooma index` reads them, `nooma find` answers a query and `nooma similar` finds what a passage is like. The window, `nooma-app`, is the same library behind one search field.

nooma searches in two halves. The half by **words** finds a document because the words of the query occur in it. The half by **meaning** finds it because it is about the same thing, whatever its words - once the [model](/nooma/reference/model/) is on the machine and `nooma index` has read the library with it; how it works is on [Search by meaning](/nooma/concepts/meaning/). `find` asks both and answers with one list, ranked from both - see [One list from two halves](/nooma/concepts/hybrid/).

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

A **source** is a folder. Everything under it that is a document - markdown and text, PDF, Word documents and EPUB books - is searched, except what is left out. A file is offered by its name - `.md`, `.markdown`, `.txt`, `.text`, `.pdf`, `.docx`, `.epub`, or no extension at all - and its first bytes decide what it is: a PDF a browser saved as `invoice` is read as a PDF, a Word document renamed to `.pdf` as a Word document, a note with no extension as text. What is left out:

- hidden files and folders, such as `.obsidian/`, `.trash/` and `.git/`;
- whatever a `.gitignore` inside a git repository excludes;
- whatever a `.nooma-ignore` file excludes - the same syntax as `.gitignore`, anywhere in the folder;
- whatever `--exclude` patterns the source was added with, in the same syntax, relative to the folder;
- text files larger than 16 MB - a log or a dump, at that size - and PDFs, Word documents and books larger than 256 MB. Their bytes are mostly pictures, which are not read, so a book of 50 MB is an ordinary one.
- a file with no extension that turns out to be no document - a picture, a program - without a word, and without being opened again until it changes. One whose name claims a document it is not - a picture named `.pdf`, an old `.doc` renamed to `.docx` - is skipped, and the report says what it is.

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

**Text** is read in the encoding it is in. A byte order mark says it, and bytes that are UTF-8 are UTF-8 - so is UTF-8 with a character cut off at the end, or one damaged byte among hundreds of letters. UTF-16 without a mark is found by reading as text in it. Anything else - a note saved in Windows-1251, a letter in KOI8-R, a file from an old DOS program in code page 866, Shift_JIS - is a legacy encoding, and the encoding is guessed by the detector Firefox uses for pages that do not name theirs. A guess is said to be one: the report counts the files read in a guessed encoding, and `--json` names them, so a wrong guess - a file found by nothing, its text gibberish - can be seen rather than wondered at. A file with bytes no text has is skipped as binary data.

**PDF** is read page by page, its text as the PDF's fonts map it - see [Chunks](#chunks) for how pages are cut. A PDF locked only against printing or copying opens as any other; one that needs a password to open is skipped, and the reason says so. A PDF with no text in it at all - a scan - is indexed by its name and the title in its metadata, so it is still found by them, and the report names it: its pages wait for OCR, which is not in this version. A PDF the reader fails on is skipped with the reason, and the rest of the update goes on. When a new nooma reads PDFs with a newer reader, the next update reads every PDF again - their bytes did not change, but what can be read out of them did - and leaves every other file as it is.

**Word documents** (`.docx`) are read for their text as Word shows it: a tracked change as it stands, without the deleted text, a text box once, a footnote right after the paragraph that points at it, under that paragraph's headings. A heading is a heading by its style, whatever language Word wrote the style's name in. A table's row is a line of its cells. A comment is a remark about the document and a header or a footer repeats on every page, so neither is read; nor are the contents Word builds from the headings, which would put every heading's words in one chunk. A Word document's title is its file name, as a PDF's is, and the title in its properties is searched as another name for it. One under a password is skipped, and the reason says so.

**Books** (`.epub`, EPUB 2 and 3) are read in their reading order, chapter by chapter, under their table of contents - see [Chunks](#chunks). A book's title is the title it gives, and its authors and its file name are searched as other names for it: a book is asked for by its author as often as by its title. A book locked by DRM is skipped, and the reason says so; one whose only cipher is the obfuscation that keeps its fonts in it is read as any other.

When a new nooma reads Word documents or books by newer rules, or guesses encodings with a newer detector, the next update reads those files again, and only those.

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
| `textless` | PDFs, Word documents and books read in this update that hold no text - scans, most likely - by path. They are in the index, found by their names. |
| `guessed` | Text files read in this update whose encoding was guessed, each with `path` and `encoding` - the guess, by its WHATWG name: `windows-1251`, `KOI8-U`, `UTF-16LE`. |
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

Prints one list, best first, one entry per document, at the place in it that answered best - `notes/plan.md:42` for a note, `books/manual.pdf · p. 12` for a PDF, the file alone for a Word document or a book, whose headings above say where. `--limit` caps it. Each entry says which halves found it: `words`, `meaning 0.84` - the cosine - or both; `meaning 0.84, another language` when the meaning alone found it in the other language than the query's, and then its fragment starts at the sentence that answered - see [Across languages](/nooma/concepts/languages/). How the two are ranked together is on [One list from two halves](/nooma/concepts/hybrid/).

**Any form of a word finds the others.** Every word is reduced to its stem before it is stored and before it is looked up, in Russian or in English by the alphabet it is written in: `договоров` finds *Договор*, `warranties` finds *warranty*. A word is judged on its own rather than by the language of the document around it, so the English terms in a Russian note are found by their English forms. `ё` and `е` are the same letter. Numbers and codes are kept as written, and `INV-2025-0114` finds that invoice number as a sequence rather than any document containing `2025`.

**Every word must occur** - in the text, the title, a heading, the tags or the links - as long as some document has them all. When none does, documents with some of the words are taken instead, and the list says so above itself and marks them `some words`. Words that most of the library holds - *the*, *for*, *и* - are left out of that weaker search: on their own they match nearly everything. Which words those are, the library counts for itself; there is no list of stop words. A document holding a small part of the question counts for little against what the meaning found.

**Markdown is read as a vault.** A heading ends one chunk and names the next; a code block is never split. From an Obsidian vault, frontmatter `tags`, `aliases` and `title`, inline `#tags` and `[[wikilinks]]` are fields of their own, so a note is found by what it is tagged with. A note is also found - more weakly - by the titles of the notes that link to it, and a note that many others link to ranks slightly higher. A link resolves within its own source, by file name or by the end of its path; a name two notes share resolves to neither. A note is linked without its `.md`, any other file with its extension, as Obsidian does: `![[manual.pdf]]` gives the PDF the linking note's title, `[[manual]]` does not.

**By meaning** comes from the vector index. When there is no model, or no vectors yet, the words answer alone and a line under the list says how to get the other half; when the vectors cover only part of the library - files changed since the last `nooma index` - the line says how many documents they do not cover yet. A document deleted since is never in the list.

`find` brings the full-text index up to date first, like `nooma index`, because answering from a stale index costs a wrong answer and the update costs a look at each file's size and time. It does not compute vectors: that is indexing, and can take an hour, so it is `nooma index` that does it. `--no-refresh` answers from what is stored.

`find --json` prints:

| Field | Meaning |
| --- | --- |
| `query` | The query, as given. |
| `refreshed` | Whether the index was brought up to date before answering. |
| `indexed_at` | When the index was last updated, in seconds since the epoch. |
| `all_words` | Whether the hits the words found hold every word of the query; `false` when no document held them all and these hold some. |
| `hits` | The documents, best first. |
| `meaning` | What the half by meaning could do: see below. |

`meaning` carries:

| Field | Meaning |
| --- | --- |
| `model` | The model that answers. |
| `state` | `ready`; `no-model` when the model is not on the machine; `no-vectors` when it is and nothing has been computed with it yet. |
| `behind` | Documents the vector index does not cover yet. |

Each hit carries:

| Field | Meaning |
| --- | --- |
| `path` | The file. |
| `kind` | What the file is, by its bytes: `markdown`, `text`, `pdf`, `docx` or `epub`. |
| `title` | The frontmatter title, the first top-level heading, or the file name. A PDF's and a Word document's is always the file name; a book's is the title it gives. |
| `headings` | The headings above the chunk shown, outermost first; in a PDF, its bookmarks leading to the page; in a book, its contents leading to the chapter and section. |
| `line` | The line the chunk starts on. In a PDF, a Word document or a book, a place counted through the text read out of it, which keeps the chunk apart from the others - not something to open it at: `page` is that, in a PDF. |
| `page` | The page the chunk is on, from 1; only in a PDF's hit. |
| `fragment` | A piece of the chunk: around the words that matched when the words carried the document, its opening when the meaning did - or, for a document found across languages, the sentence that answered and what follows. |
| `highlights` | Where the matching words are in `fragment`, as `[start, end]` byte offsets; empty when the fragment came from the meaning. |
| `closest` | Where the sentence closest in meaning to the query is in `fragment`, as `[start, end]` byte offsets; only for a document found across languages. |
| `tags` | The document's tags. |
| `modified` | When the file was last modified, in seconds since the epoch. |
| `score` | How well it answered, both halves together: comparable within one search, not across two. |
| `words` | Where the words put it, or `null` when they did not find it: `rank` from 1, `score` - the full-text score, comparable only within one search - and `share`, how much of the question its chunk holds, from 0 to 1. |
| `meaning` | Where the meaning put it, or `null`: `rank` and `score`, the cosine between the question and the chunk - 1 the same direction, 0 unrelated - which means the same from one search to the next. Both are compared without the language, when the model's was measured. |
| `across` | Whether the meaning alone found it, written in another language than the query - told by the alphabet of most of its words. |

## `similar`

```
nooma similar <FILE> [--limit <N>] [--json] [--no-refresh]
nooma similar - < passage.txt
```

Finds the documents that say what a passage says: a paragraph from a letter, a page of a note, anything you have in front of you and want the rest of your files' take on. `-` reads the passage from standard input. Any document the library reads can be the passage - a PDF, a Word document, a book - and it is read as the library reads it. When the file is itself in the library, it is left out of its own results.

Only the half by meaning answers: a paragraph's words are not a query - every one of them would have to occur. The passage is read as a passage, the way the documents were, rather than as a question; the model reads its first 512 tokens, about a page. `similar` needs the model and the vectors, and says which is missing.

`similar --json` prints `example`, the file as given or `-`; `hits`, as `find` prints them, each with only a `meaning` place - and `across` and `closest` when the document is in another language than the passage; and `meaning`, as in `find`. They are ranked as `find` ranks: by closeness, with a recently edited document a little ahead of an old one as close.

## Chunks

A document is indexed as **chunks**: runs of paragraphs under one heading, packed up to 1,500 characters. A heading always starts a new chunk, a paragraph is split only when it is longer than a chunk on its own, and a fenced code block stays whole. A result points at its best chunk, so the fragment and the line are where the answer is rather than the top of the file. The half by meaning reads the same chunks, each with its document's title and the headings above it.

In a **PDF**, a page always starts a new chunk, as a heading does: every chunk is on one page, and the page a result names is the page the answer is on. A paragraph that runs over onto the next page is two chunks. The PDF's bookmarks stand in for headings - a chunk is under the last bookmark that points at its page or an earlier one, with the bookmarks it is nested in - so a book's results say which chapter they are in. Before a page is cut, what typesetting leaves in its text is undone: a word broken across lines by a soft hyphen is joined again, ligatures and other compatibility characters become ordinary letters, and runs of spaces become one.

In a **Word document**, a heading ends a chunk and names the next, as in a note. In a **book**, a page of its reading order ends a chunk, as a PDF page does, and the table of contents stands in for headings: a chunk is under the last entry that points at its page or at a place in it before the chunk - chapters and the sections inside them alike. A book without contents is headed by its own headings. Neither has a line or a page to show; the headings above a chunk are where it is.

A PDF's title is its file name - the name you see in the folder and open it by. The title in its metadata, when it names the document rather than a file (`Microsoft Word - report.docx` does not), is searched as well, as another name for the document.
