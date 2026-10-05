---
title: Search by meaning
description: How a passage becomes a vector, how the vectors are searched and kept current, what never leaves the machine, and how the model was chosen.
---

Full-text search finds a document because the words of the question occur in it. That fails exactly when it matters: you remember what a note was about, not how you phrased it, and half your notes are in the other language. The second half of nooma finds a document by what it means. The model and the vectors it computes arrived in v0.6.0; searching with them - beside the words, as a list of its own, and by a whole passage - in v0.7.0. The two halves become one ranked list in v0.8.0.

## A passage becomes a vector

An embedding model reads a passage and returns a list of numbers - a vector - placed so that passages about the same thing land near each other, whatever words and whatever language they use. *How to get rid of limescale in the kettle* and *Накипь в чайнике: уксус или лимонная кислота* point the same way.

What the model reads for each chunk of a document is its **passage**: the document's title, the headings above the chunk, and the chunk. A chunk alone is often a paragraph that means something only under its heading - "Returns" under "Kettle" is about the kettle.

Vectors have unit length, so how close two are is the cosine of the angle between them: 1 is the same direction, 0 unrelated.

## Each model keeps its own vectors

A vector means something only next to vectors from the same weights. So the library keeps one store per model - `vectors/multilingual-e5-small/` beside the full-text index - and a second model gets a second store rather than replacing the first. Two models' answers to the same question can be compared over the same library, side by side; that is what [`nooma eval`](/nooma/reference/eval/) does.

Inside a store, a vector is kept under the hash of its passage and nothing else. Computing vectors is the slowest thing nooma does, and keying them by text means each distinct passage is computed once: a renamed file, a note copied to two folders, a document cut again by a new chunker whose chunks mostly come out the same - all find their vectors already there. Vectors are written as they are computed, so an update stopped half way keeps the half it paid for.

## Finding the closest without reading all

Comparing a question with every vector in the library is exact, and its cost grows with the library. nooma keeps the vectors of the passages the library holds now in a graph - HNSW, through [`usearch`](https://github.com/unum-cloud/USearch) - where each vector is linked to its near neighbours, and a search walks from neighbour to neighbour toward the question instead of reading them all. The answer is approximate by construction; the claim is that the approximation does not change what you see. It is measured, not assumed: against exact search over the same vectors, on the private corpus below, the graph finds the same documents - see [How the index was checked](#how-the-index-was-checked).

The graph is kept in `index.bin`, beside the store, with the chunks each passage stands for. It is derived: the store of computed vectors is what is expensive, and the graph is built again from it in seconds when it is lost, damaged, or built from another model. The file is written whole and renamed into place, so a search reading it sees one state or the next - never the graph of one with the chunks of another.

A search asks the graph for more chunks than it needs documents, because several chunks of one document can be closer than any chunk of the next, and keeps each document's closest. The hit's text comes from the full-text index, which is where the chunks are kept: the vector index knows where a chunk is, not what it says.

## Kept current, and honest about how far

The first reading of a library is long - an hour or two for a few thousand notes. Waiting for all of it before answering anything would make the half by meaning useless for that hour, so the index is saved as it goes: every minute, once it has grown by a tenth. Passages are read in the order of their documents, so what is covered is whole documents. A search answers over what is covered, and says how many documents are not yet; the window shows the count in its status line, `nooma find` in a line under the list.

Later, the index follows the full-text index: a changed file's new passages are read, a removed file's leave the graph, and a renamed file - the same passages in a new place - costs no reading at all. A document deleted since the last update is never a result, even before the vectors catch up: it could be neither shown nor opened.

In the window this all happens in the background: after the folders are read, the vectors are brought up to date while the field already answers. The model is shared between that work and your searches, borrowed by the update one small batch at a time - sixteen passages - and handed to a search that is waiting first, so a search waits at most about a batch. On the command line it is [`nooma index`](/nooma/reference/find/#index) that computes them; `find` and `similar` answer from what is stored.

## Searching by example

A question is a few words; sometimes what you have is a paragraph - from a letter, from a note open in another window - and the question is what else you wrote about this. Paste it into the window's field, or give it to [`nooma similar`](/nooma/reference/find/#similar), and the documents closest to it in meaning come back. A paste is taken as a passage rather than a query when it has more than one line or is longer than 140 characters.

A passage is compared with passages: it is read the way the documents were, with the model's passage prefix, not as a question. The exact half is not asked - a paragraph's words are not a query, and every one of them would have to occur.
## What never leaves the machine

The model runs on your CPU, through ONNX Runtime linked into nooma. Fetching it - [`nooma model fetch`](/nooma/reference/model/) - is the one thing nooma does over the network, only when you run it, and it downloads the model's files and sends nothing.

This is held by structure, not by promise. The code that fetches lives in a crate of its own, `nooma-fetch`; the library that reads your files, `nooma-core`, has no HTTP client and no TLS among its dependencies at all - and a test asks cargo for that crate's full dependency graph and fails if one appears, checking the fetching crate the same way so that the test cannot pass on a graph it failed to read.

Models are pinned: the commit of their repository on the Hugging Face Hub, and the size and SHA-256 of every file. A download that differs by one byte is refused, so the model on your machine is the one that was measured.

## How the model was chosen

By measuring, on two sets of questions whose answers were known, with [`nooma eval`](/nooma/reference/eval/). Three multilingual models were in the running - English-only ones were ruled out first.

**A synthetic bilingual corpus**, in the repository at `crates/nooma-core/tests/meaning`: eight everyday subjects, each with the answer, a near miss from the same subject, and a decoy that shares its words and not its meaning - a dripping tap beside a tower crane, *кран* both. Thirty-two questions, most of them asked in the other language or in words the answer does not use.

**A private corpus** of the author's own: 936 notes and documentation pages, 12,889 chunks, mostly Russian with English throughout, and seventy questions written the way someone half-remembering a note would type them. Nothing of it is in the repository; only these numbers are.

MRR - the mean of one over the rank of the answer, see [eval](/nooma/reference/eval/#what-is-measured):

| | Synthetic | Private | Private, same language | Private, across languages | Passages a second |
| --- | --- | --- | --- | --- | --- |
| Full-text (for scale) | 0.48 | 0.26 | 0.49 / 0.23 | 0.00 | - |
| `multilingual-e5-small` | 0.63 | **0.39** | **0.55 / 0.81** | 0.00 | 3-8 |
| `paraphrase-multilingual-minilm-l12-v2` | 0.84 | 0.28 | 0.27 / 0.49 | 0.21 / 0.23 | 12-13 |
| `bge-m3` | **0.87** | - | - | - | 0.6 |

Same language is Russian questions about Russian notes, then English about English; across languages is English about Russian, then Russian about English. Speed is on four threads of a laptop CPU - the share nooma allows itself while you work.

What the numbers say:

- **`multilingual-e5-small` is the best of the fast models on the real archive**, and best exactly where the words fail: a question about a note in words the note does not use (Russian, hit@3 0.72 against 0.48 for full-text) and a question carrying an identifier (hit@3 0.75 against 0.42). It is the model nooma uses.
- **It does not cross languages.** Asked in English about a Russian note, it ranks the near miss in English above the answer - on the synthetic set it still finds the answer among the first ten, on the real archive it does not. Search across languages is its own step, v0.9.0, measured with these same sets; a model choice was not going to deliver it.
- **`paraphrase-multilingual-minilm-l12-v2` crosses languages, a little**, and does worse than plain full-text within one. It also reads only the first 128 tokens of a chunk.
- **`bge-m3` is the strongest on the synthetic set, and twenty times slower.** At 0.6 passages a second a two-thousand-note vault takes most of a day to read. For a tool that runs on your laptop beside your work, that rules it out whatever it scores; it is in the catalogue for measuring with `nooma eval`.

A better multilingual model will come. Changing to it costs a catalogue entry and one pass of computing vectors, and is measured on the same questions before it is made.

## How the index was checked

On the private corpus above - grown since to 984 documents, 13,463 chunks and 13,260 distinct passages - `nooma eval` asked each of the seventy questions of the graph and of exact search over the same vectors. They found the answer to the same questions at the same ranks: MRR 0.39 and hit@3 0.49 both, in every group alike, and what exact search measured when the model was chosen. Three questions of seventy differed at all, and only in the order of documents below the answer. A question took 35 ms either way, nearly all of it the model reading the question.

Building the graph again from the stored vectors - after the index is lost, damaged or mostly rewritten - took nine seconds for those 13,260 passages, and an update that finds nothing changed takes a fifth of a second. Reading the passages with the model the first time used at most 1.5 GB of memory: the batches are small, sixteen passages, against the 2.1 to 2.7 GB that batches of thirty-two took. Over random vectors - the hardest case for a graph - its default search width missed one in ten of the closest ten; nooma searches four times wider, and a test holds that against exact search.
