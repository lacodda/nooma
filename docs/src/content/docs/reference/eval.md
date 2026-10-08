---
title: eval
description: Measure search against questions whose answers you know - by words, by meaning and by both as find ranks them, on your own folders.
---

A search that feels better after a change has not been shown to be better. `nooma eval` asks every question in a query set of the full-text index, of one or more models, and of the two together as `find` asks them, and counts where the known answer landed. It is how nooma's model was chosen, and how a change to chunking or ranking is judged - on your folders, with questions you wrote.

```
nooma eval <QUERIES> [--model <ID>]... [--json] [--no-refresh]
```

| Argument | Meaning |
| --- | --- |
| `<QUERIES>` | The query set, a JSON file - see below. |
| `--model <ID>` | A model to measure beside the full-text index; repeat it to compare several. The one nooma uses when left out. Each must be on this machine: see [`model fetch`](/nooma/reference/model/). |
| `--json` | Print one JSON document instead of a report. |
| `--no-refresh` | Answer from the stored index without bringing it up to date first. |
| `--store <DIR>`, `--models <DIR>` | As for [`find`](/nooma/reference/find/) and [`model`](/nooma/reference/model/). |

As `find` does, `eval` first brings the full-text index up to date. Then, for each model, it computes vectors for every passage that does not have one from that model yet, and keeps them: the first run over a large library takes a while, the second costs only the questions. Every model is checked for before any is run - finding the third one missing after the first two spent an hour would waste the hour.

## The query set

```json
{
  "queries": [
    { "query": "how do I get my money back for the kettle",
      "expect": ["home/receipts.md"],
      "group": "en→ru" }
  ]
}
```

| Field | Meaning |
| --- | --- |
| `query` | What a person would type. |
| `expect` | The documents that answer it, by path inside their source folder, with `/` between folders. Any one of them counts. |
| `group` | Optional. Results are reported per group as well as overall - by language pair, say, or by kind of question. |

Every expected document must be in the library. A misspelt path would count as a miss for every engine and read as all of them being worse, so the set is refused and the unknown paths named instead.

Questions worth writing are the ones search finds hard: a description of a note in words it does not use, a question in one language about a document in the other, a half-remembered situation. Questions made of a document's own title words measure little.

## What is measured

Each question is asked of each engine, and the rank of the first expected document among the first ten results is recorded.

Every model is measured three times. `<model>` asks its vector index alone - a graph that finds the closest vectors without reading them all, and is approximate by construction. `<model> exact` compares the question with every vector instead; the two rows side by side are what the approximation costs on your questions - nothing, when they agree. See [Search by meaning](/nooma/concepts/meaning/#finding-the-closest-without-reading-all). `<model> hybrid` asks the way `find` does: the words and the model's vector index, ranked as one list - see [One list from two halves](/nooma/concepts/hybrid/). It is the row that says how well `find` answers.

| Measure | Meaning |
| --- | --- |
| hit@1 | The share of questions answered by the first result. |
| hit@3 | The share answered within the first three. |
| hit@10 | The share answered within the first ten. |
| MRR | The mean of one over the rank of the first answer, zero when it is not in the first ten. It rewards putting the answer first, not just near. |

```
$ nooma eval queries.json
32 questions · 24 documents · 64 chunks

                              hit@1  hit@3  hit@10    MRR  ms/question
fulltext                       0.47   0.50    0.53   0.48          1.5
multilingual-e5-small          0.50   0.69    0.94   0.63         24.8
multilingual-e5-small exact    0.50   0.69    0.94   0.63         24.9
multilingual-e5-small hybrid   0.50   0.66    0.94   0.59         26.0
```

That is the bilingual test corpus in nooma's repository, `crates/nooma-core/tests/meaning`, measured on a laptop. It is built against the words - every answer has a decoy that shares them - so there the list from both halves trails the meaning alone; on a real archive it leads both, see [One list from two halves](/nooma/concepts/hybrid/#how-it-was-measured).

Then the same table per group, how long each model took to compute its vectors, and the questions each engine missed with what it found instead.

## `--json`

| Field | Meaning |
| --- | --- |
| `queries` | Questions in the set. |
| `documents` | Documents in the library. |
| `chunks` | Chunks in the library. |
| `engines` | One entry per engine, full-text first, with the fields below. |
| `engine` | `fulltext`; a model's id, searched through its vector index; the id followed by ` exact`, the same vectors compared one by one; or by ` hybrid`, the words and the vector index ranked as one list, as `find` ranks them. |
| `overall` | The measures over every question: `queries`, `hit_at_1`, `hit_at_3`, `hit_at_10`, `mrr`. |
| `groups` | The same measures per group, by group name. |
| `outcomes` | One entry per question: `query`, `group`, `rank` (from 1, or `null` when not in the first ten) and `top`, the first three documents found. |
| `query_ms` | The mean time to answer one question; for a model it includes turning the question into a vector. |
| `vectors` | One entry per model: `model`, `load_ms` - how long loading it took - and `report`, below. |
| `report` | What bringing its vectors up to date did: `model`, `chunks`, `passages` (distinct texts - identical chunks share a vector), `embedded` (passages computed in this run), `dropped` (vectors of passages that no longer exist, removed), `rebuilt` (why the store was started again, or `null`), `took_ms`, and `embed_ms` - the time the model itself spent. |
