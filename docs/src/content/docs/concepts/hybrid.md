---
title: One list from two halves
description: How the words and the meaning are ranked together, why a word in common counts for little, what freshness adds, and how the balance was measured.
---

nooma searches two ways. The words find a document because the words of the question occur in it: the invoice by its number, the error by its code, the note by the name you remember. The [meaning](/nooma/concepts/meaning/) finds it because it is about the same thing, in whatever words: the note you wrote about the kettle when you ask about limescale. Each fails where the other works, so neither alone is the product. Since v0.8.0, `find` and the window ask both and show one list - one row per document, best first, each row saying which half found it.

## By rank, not by score

A full-text score and a cosine are not on one scale. The first grows with the length of the question and the rarity of its words; the second sits in a narrow band that differs from model to model. Adding them would let whichever happened to run larger decide. What both halves agree on is order, so a document earns from each half that found it a share that falls with its place in that half's list - reciprocal rank fusion - and the list is ordered by the sum. A document both halves put near the top is the strongest answer there is; a document only one half found still gets its place.

Each half is asked for thirty documents at the least, more than are shown: a document the words put twentieth and the meaning third belongs near the top, and is only found when both lists reach that far.

## A word in common is not an answer

When no document holds every word of the question, the words take the documents holding some of them, and the list says so. For a question asked in words the answer does not use, those are the documents that share a word with it and nothing else - a tower crane for a dripping tap, *кран* both. Ranked like the rest, they push the document the meaning found down and out of sight.

So what the words add for a document is weighed by how much of the question its chunk holds, squared: all of it counts fully, half of it a quarter, one word of six next to nothing. A chunk that holds the whole question - a code, a name, a phrase - still leads what the meaning merely put first.

Words that most of the library holds - *the*, *for*, *и* - are left out of both the weaker search and the count: on their own they match nearly every document, and holding them says nothing. Which words those are, the library counts for itself, a word at a time; there is no list of stop words per language to keep.

## Fresh, a little

A note edited this week is a little more likely to be the one you mean than one untouched for a year. A recently modified document gets up to a tenth more, and the bonus halves every thirty days. A little is the point: freshness settles a near tie between two documents that answer about as well, and does not lift a weak answer over a good one. A clock set wrong, stamping a file in the future, counts as now and no more.

## What a row shows

The chunk shown for a document is the one from the half that carried it: the words' chunk with the matching words marked, when the words carried it; the meaning's chunk, from its opening, when the meaning did. The row says which halves found it - the window with a badge for each, `find` in brackets after the title: `words`, `some words` when no document held them all, `meaning 0.84` with the cosine, or both.

The score of a row is the sum, freshness included. It orders one list; it is not comparable between two searches, and the window draws it as a bar against the first row rather than as a number.

## No dial

The balance between the words and the meaning is not a setting. A dial would hand the measurement to the person searching, who would have to find out by trying what the query sets below already say. The numbers - how fast a place stops counting, how much the words weigh against the meaning, how much a share of the question counts, how much freshness adds - were measured, and changing any of them is measured the same way.

## How it was measured

With [`nooma eval`](/nooma/reference/eval/), on the two query sets that chose the model. First the halves' answers to every question were taken down, thirty documents each; then every way of ranking them together was scored against the known answers: reciprocal rank fusion with the place it stops counting from 1 to 60, the words weighed from three quarters to twice the meaning, the share of the question counted plainly, squared and cubed, and a weighted sum of scores normalized per search beside it. The settings that scored best form a broad flat region - every neighbour of the chosen point within a couple of hundredths - and nooma uses its middle rather than its single best point, which on seventy questions would be fitting noise.

MRR - the mean of one over the rank of the answer:

| | Words | Meaning | One list |
| --- | --- | --- | --- |
| Private archive, all 70 questions | 0.27 | 0.38 | **0.43** |
| - Russian about Russian (25) | 0.49 | 0.55 | **0.64** |
| - English about English (8) | 0.21 | 0.76 | **0.79** |
| - carrying an identifier (12) | 0.38 | 0.58 | **0.62** |
| - across languages (25) | 0.00 | 0.00 | 0.00 |
| Synthetic set, 32 questions | 0.48 | **0.63** | 0.59 |

The private archive is the author's own: 1,001 notes and documentation pages, 13,767 chunks, mostly Russian with English throughout, and questions written the way someone half-remembering a note types them. Nothing of it is in the repository; only these numbers are. On it the one list answers better than either half in every group where either answers at all, and puts the answer first a quarter more often than the meaning alone (hit@1 0.36 against 0.29).

The synthetic set in the repository is built against the words: every answer has a decoy that shares them, and half its questions are asked in the other language. There the list trails the meaning alone, by placing the answer a little lower among the first ten - but keeps it among the first ten as often (hit@10 0.94 both). Without weighing the words by how much of the question they hold, that was 0.66: the decoys pushed the answer off the screen. A test holds the list to its floors on this set, with the real model.

Asking both halves costs little more than asking the meaning: on the private archive a question took 23 ms through the one list against 12 ms through the meaning alone, the model's reading of the question included - a release build on four threads of a laptop CPU.

Freshness was measured last, on the same sets, whose questions were written with no time in mind: the bonus nooma uses moved MRR by less than a hundredth there, and stronger ones - a fifth or more, halving within a week - began to cost a few hundredths. Across languages the list is no better than its halves, because neither half can do it yet: that is [v0.9.0](/nooma/#status).
