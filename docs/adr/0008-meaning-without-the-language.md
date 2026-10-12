# 0008 — Meaning without the language: two directions measured on parallel pairs, taken out before comparing

Date: 2026-10-12
Status: accepted

## Context

v0.6.0 chose `multilingual-e5-small` (ADR 0005) knowing its price: on the private archive a question in one language found the answer in the other for none of 25 questions. The plan's premise for v0.9.0 - "the model already can" - was false, and v0.8.0's one list could not help, since neither half crossed languages.

The cause is measurable. A multilingual model places a text by what it says and, apart from that, by the language it says it in. The second part is small and the same for every text of a language, so it adds up: a Russian question sits a little closer to every Russian note than to any English one, and in a library of a thousand documents the English answer never reaches the first ten.

v0.6.0 tried the textbook fix - subtract from each vector the mean vector of its alphabet - and rejected it: it lifted the synthetic set from 0.63 to 0.86 and lowered the private archive from 0.39 to 0.36. In a real library the language goes with the subject - the English documents are manuals, the Russian ones notes - and the mean of an alphabet carries the subject away with the language.

Three ways were open: estimate the language better and take out only it; translate the question, which needs a translation model on the machine and a second network download; or change the model, which is the owner's question still open (bge-m3 reads twenty times slower; multilingual-e5-base was not measured).

## Decision

**The language is estimated on parallel pairs.** `crates/nooma-core/data/parallel-en-ru.json` holds 480 pairs of texts that say the same thing in English and Russian, written for nooma: four areas (software, home life, people and health, the world and culture), each with search-box queries, sentences, short passages and markdown notes, half of them first written in each language. The difference between the two vectors of a pair is the language and nothing else - the subject is the same on both sides.

**Two directions are taken out.** Every text of the set is read as a query and as a passage; the strongest two directions of the pairs' differences - the leading eigenvectors of their second-moment matrix, found by repeated multiplication - are the model's language. The first stands far above the rest, the second clearly apart from the third (singular values 7.1, 2.5, 1.7, 1.6, ...).

**The graph holds the meaning; the store keeps what the model computed.** Before a vector joins the graph, its components along the two directions are removed and it is scaled back to unit length; a query, or a passage searched by example, is treated the same before the graph is asked. `vectors.bin` is untouched, so nothing is computed again: `index.bin` names the space it was built in, and an index from another space is built again from the store in seconds. Its format goes from 1 to 2.

**The directions are a property of the model, kept in the catalogue** beside its pooling and prefixes: `data/axes/<model>.json`, with the hash of the parallel set and the model's recipe they were measured with. The `language_axes` example measures them. A unit test, with no model, holds every entry to the current set and recipe; a test with the real model measures them again and holds the catalogue to what it gets. A model whose language was not measured - the two other catalogue entries - compares its vectors as they come.

**A row found across languages says why.** When the meaning alone found a document, written in another alphabet than the question - told apart by words, as the full-text half tells them - the row is marked `another language`, and its fragment is moved to the sentence of the chunk closest to the question, compared as the index compares, and marked. Only those rows: they are the ones with nothing else to show, and asking every row the meaning found made `find` take seconds longer.

## Why these numbers

All measured with the private archive's 70 questions (1,001 documents) and the synthetic set's 32, first over dumped vectors by exact search, then through `nooma eval`.

- **How many directions.** One, two and four score alike on the private archive (list MRR 0.457, 0.464, 0.459); eight and more begin to take the meaning too. Two is the middle of the flat region.
- **Taken out of both sides, or only the question's.** Removing them from the question alone leaves the passages' lengths uneven and scores 0.39; both sides, renormalized, 0.46.
- **Two lists of meaning instead of one** - the vectors as they come beside the vectors without the language - scored 0.41 to 0.44, and crossed no languages: the list as it comes brings the decoys in the question's language back.
- **LEACE**, measuring the language along the direction that best tells the languages apart and removing it along the difference of means, scored 0.455 to 0.465: no better than the plain projection, and it needs a covariance.
- **Taking out part of the language** (half to three quarters of it) scored lower than all of it.
- **Stability.** Any half of the pairs gives 0.448 to 0.465; the planes measured on two halves agree to 0.99 and 0.96; 120 pairs of one area alone are enough.
- **The fusion of ADR 0007** was measured again over the new meaning: its constants (`k` 8, weight 1.5, power 2) stay on the plateau, 0.464 against the best point's 0.468, and are kept.

Through `nooma eval`, MRR:

| | v0.8.0 | v0.9.0 |
| --- | --- | --- |
| Private archive, one list | 0.43 | **0.45** |
| - answer in the first ten | 0.56 | **0.67** |
| - across languages (25), first ten | 0 | **10** |
| - Russian about Russian (25) | **0.64** | 0.57 |
| - English about English (8) | **0.79** | 0.77 |
| - carrying an identifier (12) | 0.62 | 0.62 |
| Private archive, meaning alone | 0.38 | **0.40** |
| Synthetic set, one list | 0.59 | **0.76** |
| Synthetic set, meaning alone | 0.63 | **0.86** |

## Consequences

Positive:

- A question finds the answer in the other language: ten of the twenty-five private questions that cross languages now have it in the first ten, against none; on the synthetic set every answer is in the first three.
- Nothing is computed again: the store is kept, the graph rebuilt from it.
- A row found across languages shows the sentence that answered instead of the opening of its chunk.

Negative:

- Within a language the list is a little weaker, because answers in the other language now compete: on the private archive two questions in their own language - one Russian, one English - fell out of the first ten while ten across languages came in, and Russian MRR went from 0.64 to 0.57.
- Across languages the answer is found, but rarely first (hit@1 0.08): the decoy that shares a word with the question, found by both halves, still stands above it. The model's alignment is the limit; a stronger model is the owner's question.
- The vector index format goes from 1 to 2: an existing index is rebuilt from the stored vectors on the next update.
- The parallel set is maintained with the code; changing a pair, or a model's recipe, means measuring that model's language again, and the tests say so.
- Explaining a row costs the model a read of its chunk's sentences: 60 to 260 ms each on four threads, by the chunk's length.
