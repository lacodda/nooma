# 0007 — One list: reciprocal rank fusion, weighed by how much of the question the words hold

Date: 2026-10-08
Status: accepted

## Context

v0.7.0 searched both halves and showed two lists: the documents holding the words, and the documents close in meaning. ADR 0001 made the hybrid the product; two lists side by side were the step before it. The window had to cap each list at five rows so the second stayed on screen, and a person had to read two rankings and merge them by eye.

Three things had to be settled to show one list:

1. **How two rankings become one.** A full-text score grows with the length of the query and the rarity of its words, without a ceiling; a cosine from `multilingual-e5-small` sits in a narrow band, around 0.75 to 0.9 for anything related at all. Neither scale means anything to the other.
2. **What to do with the full-text half's fallback.** When no document holds every word, the full-text half takes documents holding any of them. For most real questions - a sentence, half-remembered - that fallback is the normal case: 64 of the 70 questions of the private set, and 27 of 32 of the synthetic one. For a question asked in other words than the answer's, it returns the documents that share a word with the question and nothing else - and in v0.7.0 it also returned every document sharing a *the*.
3. **The plan's "freshness as a signal"**: a recently edited note as a little more likely to be the one meant.

The plan also named "a setting for the balance between precision and meaning".

## Decision

**Reciprocal rank fusion.** Each half is asked for at least thirty documents. A document earns `1 / (8 + rank)` from the meaning's list and `1.5 · share² / (8 + rank)` from the words' list, where `share` is the part of the question's distinctive words its chunk holds - 1 whenever every word matched. The list is ordered by the sum, ties by path.

**The words weigh by how much of the question they hold.** The share counts the query's terms, as the index holds them, against every searched field of the chunk. A word in common with the question counts for little; the whole question counts fully, and outweighs what the meaning merely put first.

**Words most of the library holds do not count.** In the fallback search and in the share, a plain word that more than half the chunks hold is left out, judged by the index's own document frequencies. There is no stop-word list per language; a query of nothing but such words is kept whole, and a word with syntax in it - a code, a quote - is never left out.

**The answer says when it is the weaker one.** `Matched::all_words` is false when no document held every word; `find` says so above the list, its rows say `some words`, and the window's badge and heading say the same.

**Freshness is a multiplier of at most 1.1,** falling by half every thirty days of the file's modification time, which the full-text index now stores with every chunk.

**No setting.** The balance is a measured constant.

**The chunk shown** is the one from the half that contributed more to the document's score: the words' with its highlights, or the meaning's.

## Why these numbers

The halves' answers to every question of both query sets were recorded, thirty deep, and every combination scored against the known answers: RRF with `k` from 1 to 60; the words' weight from 0.5 to 2; the share counted not at all, plainly, squared and cubed; the fallback down-weighted as a whole instead of by share; and a weighted sum of scores normalized per search (by the top full-text score, and the cosine against the list's floor or a fixed 0.7).

- Without the share, every fusion lost the synthetic set's cross-language answers out of the first ten (hit@10 0.94 for the meaning alone, 0.56 to 0.75 fused): the fallback's decoys, found by both halves, outranked them. Down-weighting the fallback as a whole saved the synthetic set and cost the private one, where the fallback is how most questions are answered.
- With the share squared, the region `k` 5 to 10, weight 1.25 to 2, power 1.5 to 2.5 scores alike: private MRR 0.41 to 0.43, synthetic 0.57 to 0.62, synthetic hit@10 0.91 to 0.94. The middle of it - `k` 8, weight 1.5, power 2 - is used, not the single best point, which on seventy questions would be noise.
- The normalized weighted sum was no better at its best and depends on the spread of cosines, which is the model's, not the library's.
- Thirty candidates per half scored the same as fifty, and ten less on the private set.
- Freshness at 0.1 with a thirty-day half-life moved MRR by under a hundredth on sets written with no time in mind; 0.2 to 0.3 with a half-life of a week cost up to 0.03.

Measured with `nooma eval`, which now reports `<model> hybrid` beside each half - private set, 1,001 documents: words 0.27, meaning 0.38, one list 0.43; synthetic: 0.48, 0.63, 0.59.

A dial for the balance was not added: the two behaviours are not needed at once, and the dial would hand the measurement to the person searching. A query that wants only exact words is answered by the share - every word matched weighs the most - and the operators of v0.18.0 will be the explicit way.

## Consequences

Positive:

- One list, one row per document, each row saying which halves found it.
- Better than either half on the private set in every group where either answers; on the synthetic set the answer stays in the first ten as often as with the meaning alone.
- The fallback no longer floods the list with documents sharing only common words.
- The window shows the words' answer at once and the whole answer when the model has read the question, without two lists to keep apart.

Negative:

- The full-text index format goes from 1 to 2: every chunk stores its file's modification time and every searched field. Existing indexes are rebuilt on the next update - seconds to a minute; the vectors are kept, since they are keyed by text.
- A share is computed for each fallback hit by analyzing its chunk: tens of microseconds per hit.
- Across languages the list is as weak as its halves; that is v0.9.0.
- The constants were fitted on 102 questions. They are measured again whenever chunking, the analyzer or the model changes.
