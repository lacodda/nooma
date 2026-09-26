---
title: eval
description: Measure search against questions whose answers you know - by words and by meaning, side by side, on your own folders.
---

A search that feels better after a change has not been shown to be better. `nooma eval` asks every question in a query set of the full-text index and of one or more models, and counts where the known answer landed. It is how nooma's model was chosen, and how a change to chunking or ranking is judged - on your folders, with questions you wrote.

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

| Measure | Meaning |
| --- | --- |
| hit@1 | The share of questions answered by the first result. |
| hit@3 | The share answered within the first three. |
| hit@10 | The share answered within the first ten. |
| MRR | The mean of one over the rank of the first answer, zero when it is not in the first ten. It rewards putting the answer first, not just near. |

```
$ nooma eval queries.json --model multilingual-e5-small --model paraphrase-multilingual-minilm-l12-v2
32 questions · 24 documents · 64 chunks

                                       hit@1  hit@3  hit@10    MRR  ms/question
fulltext                                0.47   0.50    0.53   0.48          1.5
multilingual-e5-small                   0.50   0.69    0.94   0.63         24.8
paraphrase-multilingual-minilm-l12-v2   0.72   0.97    1.00   0.84         22.6
```

That is the bilingual test corpus in nooma's repository, `crates/nooma-core/tests/meaning`, measured on a laptop.

Then the same table per group, how long each model took to compute its vectors, and the questions each engine missed with what it found instead.

## `--json`

| Field | Meaning |
| --- | --- |
| `queries` | Questions in the set. |
| `documents` | Documents in the library. |
| `chunks` | Chunks in the library. |
| `engines` | One entry per engine, full-text first, with the fields below. |
| `engine` | `fulltext`, or a model's id. |
| `overall` | The measures over every question: `queries`, `hit_at_1`, `hit_at_3`, `hit_at_10`, `mrr`. |
| `groups` | The same measures per group, by group name. |
| `outcomes` | One entry per question: `query`, `group`, `rank` (from 1, or `null` when not in the first ten) and `top`, the first three documents found. |
| `query_ms` | The mean time to answer one question; for a model it includes turning the question into a vector. |
| `vectors` | One entry per model: `model`, `load_ms` - how long loading it took - and `report`, below. |
| `report` | What bringing its vectors up to date did: `model`, `chunks`, `passages` (distinct texts - identical chunks share a vector), `embedded` (passages computed in this run), `dropped` (vectors of passages that no longer exist, removed), `rebuilt` (why the store was started again, or `null`), `took_ms`, and `embed_ms` - the time the model itself spent. |
