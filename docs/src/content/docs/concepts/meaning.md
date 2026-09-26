---
title: Search by meaning
description: How a passage becomes a vector, why each model keeps its own, what never leaves the machine, and how the model was chosen.
---

Full-text search finds a document because the words of the question occur in it. That fails exactly when it matters: you remember what a note was about, not how you phrased it, and half your notes are in the other language. The second half of nooma finds a document by what it means. Its first piece - the model, and the vectors it computes - arrived in v0.6.0; searching with it arrives in v0.7.0, and the two halves become one ranked list in v0.8.0.

## A passage becomes a vector

An embedding model reads a passage and returns a list of numbers - a vector - placed so that passages about the same thing land near each other, whatever words and whatever language they use. *How to get rid of limescale in the kettle* and *Накипь в чайнике: уксус или лимонная кислота* point the same way.

What the model reads for each chunk of a document is its **passage**: the document's title, the headings above the chunk, and the chunk. A chunk alone is often a paragraph that means something only under its heading - "Returns" under "Kettle" is about the kettle.

Vectors have unit length, so how close two are is the cosine of the angle between them: 1 is the same direction, 0 unrelated.

## Each model keeps its own vectors

A vector means something only next to vectors from the same weights. So the library keeps one store per model - `vectors/multilingual-e5-small/` beside the full-text index - and a second model gets a second store rather than replacing the first. Two models' answers to the same question can be compared over the same library, side by side; that is what [`nooma eval`](/nooma/reference/eval/) does.

Inside a store, a vector is kept under the hash of its passage and nothing else. Computing vectors is the slowest thing nooma does, and keying them by text means each distinct passage is computed once: a renamed file, a note copied to two folders, a document cut again by a new chunker whose chunks mostly come out the same - all find their vectors already there. Vectors are written as they are computed, so an update stopped half way keeps the half it paid for.

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
