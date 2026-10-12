---
title: Across languages
description: Why a question in one language missed the answer in the other, how nooma takes the language out of the meaning, what a row found across languages shows, and what it costs.
---

Half of what many people write is in one language and half in another: notes in Russian, manuals and code in English, the same subject in both. Since v0.9.0 a question in either finds the document in the other, with the half by [meaning](/nooma/concepts/meaning/). The half by words cannot - *накипь* and *limescale* share no letters - so across languages the meaning is the whole answer.

## Why the model alone did not

A multilingual model places a text by what it says and, apart from that, by the language it says it in. The second part is small, but it is the same for every text of one language, so it adds up: a Russian question is a little closer to every Russian note than to any English one. In a library of ten notes that is noise. In a library of a thousand, the English document that answers a Russian question sits below dozens of Russian ones that merely share its subject, and never reaches the first ten. Measured on the author's archive before v0.9.0, `multilingual-e5-small` found the answer in the other language for none of twenty-five such questions.

## Taking the language out

A pair of texts that say the same thing in two languages differ in their language and in nothing else. nooma keeps 480 such pairs - queries, sentences, paragraphs and notes on every kind of subject, written for it - and from the differences between the two vectors of each pair finds the directions the model carries its language along. Two stand far above the rest. Before two vectors are compared, both lose their components along those two directions and are scaled back to unit length: what is compared is the meaning that is left.

Two things follow:

- **Nothing is computed again.** The vectors the model computed are kept as they were; the graph the search walks holds them without the language, and is built again from the kept vectors - seconds - when it was built before.
- **The directions belong to the model.** They are measured once per model and kept in its entry of the catalogue, with the pairs and the model's recipe they were measured with; a test fails if either has changed since. A model whose language was not measured compares its vectors as they come.

The obvious shortcut - subtracting the average vector of each alphabet - was tried first, in v0.6.0. It takes the subject away with the language, because in a real library they go together: the English documents are manuals, the Russian ones notes. The two texts of a pair share their subject, so their difference cannot carry it away. The measurements, and the ways that scored worse, are in `docs/adr/0008-meaning-without-the-language.md`.

## What a row found across languages shows

A row the meaning alone found, written in another language than the question, has nothing that looks like the question in it. So it says so - the window marks it **another language**, `find` writes `meaning 0.84, another language` - and shows the place that answered: the sentence of its chunk closest to the question, compared the same way, with its fragment starting there. The window marks that sentence in the meaning's colour, as it marks the words that matched in the words'; `find --json` gives it as `closest`, byte offsets into the fragment.

Finding that sentence costs the model a read of every sentence of the chunk - from a twentieth of a second for a short one to a quarter for a long one - so it is done only for these rows: in the window one row at a time after the list is shown, top first; on the command line for the rows it prints. A row in the question's own language shows the opening of its chunk, as before.

Which language a text is in is told by its words' alphabets, as the full-text half tells them: Cyrillic is Russian, Latin is English, and the alphabet most of the words are written in decides. A Russian note full of English terms is a Russian note.

## What it costs, measured

On the same two query sets as [One list from two halves](/nooma/concepts/hybrid/), MRR:

| | Before (v0.8.0) | Now |
| --- | --- | --- |
| Private archive, one list | 0.43 | **0.45** |
| - the answer among the first ten | 0.56 | **0.67** |
| - across languages (25 questions), answered in the first ten | 0 | **10** |
| - Russian about Russian (25) | **0.64** | 0.57 |
| - English about English (8) | **0.79** | 0.77 |
| - carrying an identifier (12) | 0.62 | 0.62 |
| Synthetic set, one list | 0.59 | **0.76** |
| Synthetic set, meaning alone | 0.63 | **0.86** |

Across languages the answer is now found, and rarely first: the document in the question's own language that shares a word with it, found by both halves, still stands above it. Within a language the list gave a little back - two questions of thirty-three fell out of the first ten - because an answer in the other language now competes with it. That is what a model of this size can do; a larger one would align the languages better and read the library many times slower, and is measured with [`nooma eval`](/nooma/reference/eval/) before it is chosen.

The directions were measured once more on each half of the pairs alone: either half gives nearly the same two (they agree to 0.99 and 0.96) and answers the questions above within two hundredths of the whole set. A hundred and twenty pairs on one subject were enough. The 480 are there so that no subject's own direction is mistaken for the language's.
