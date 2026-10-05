# 0006 — The vector index is a graph derived from the store, written as one file

Date: 2026-10-04
Status: accepted

## Context

v0.6.0 left every computed vector in a per-model store, `vectors/<model>/vectors.bin`, keyed by the hash of the passage, and searched it exactly: every chunk read from the full-text index, every vector read into memory, the query compared with each. That was right for choosing a model and wrong for a search field:

1. **Opening was the cost.** Reading every chunk out of the full-text index to learn which vector belongs to which file took longer than any search, and a window answering every keystroke would pay it each time an update changed anything.
2. **It refused to answer until it could answer about everything.** A chunk without a vector made the whole search an error. The first reading of a few thousand notes takes one to two hours on a laptop; for those hours there would be no search by meaning at all, and after every edit until the next update.
3. **Exact search grows with the library.** At thirty thousand chunks it is still milliseconds; at a million - the size a code base or a mail archive reaches - it is not.

ADR 0001 named `usearch` for the approximate half, and three questions came with it: what the graph is keyed by and where it lives, how it stays consistent with a full-text index that changes under it, and how a window that keeps it open coexists with a command line that rewrites it.

## Decision

**The store stays the record of what was computed; the graph is derived from it.** `vectors.bin` keeps every vector the model computed, appended as computed. `index.bin` holds the graph over the passages the library holds now, and beside each passage the chunks it stands for - a document and a line. The graph is built again from the store in seconds when it is missing, unreadable, or from another recipe; the store is never thrown away to fix the graph.

**The graph is keyed by its own ids, and the passages beside it by hash.** A passage's id is given when it enters the graph and kept until it leaves. Compacting the store renumbers its records and does not touch the graph. An update removes the passages no chunk has any more, adds the ones it computes, and adds any the store has that the graph lacks - the vectors of an update that stopped before saving. When more than a quarter of the graph would be removed, it is built again instead, since a graph full of holes searches worse.

**One file, written whole and renamed into place.** A head (model, recipe, length, and the documents with the hash each was read at and whether it is complete), the passages with their chunks, and the graph as `usearch` serializes it. A reader sees one state or the next, never one's graph with another's chunks. It is read into memory rather than memory-mapped: Windows cannot replace a mapped file, and a window holding the index would block every update made from the command line.

**An index can be partial, and says by how much.** A long update saves the index every minute once it has grown by a tenth, and saves it when it stops for any reason. Passages are read in the order of their documents - sorted by length only within windows of 256, which keeps the padding saving - so what is covered is whole documents. A search answers over what is covered and reports the documents it does not cover: not in the index, in it in part, or in it as an older version of the file. A document deleted since the index was saved is never a result. The evaluation still refuses a partial index - a measurement over part of the library would read as one over the whole.

**Only the index is opened to search, and the model only to turn a question into a vector.** The catalogue entry names the vectors - model, recipe, length - without loading weights, so whether there is anything to search is known before the model is loaded. The text a hit shows comes from the full-text index by path and line, where the chunks are kept; the vector index keeps no copy.

**The approximation is measured on every evaluation.** `nooma eval` reports each model twice: through the graph, as `find` searches, and by exact search over the same vectors. A recall test against exact search guards the graph's settings in the test suite.

**The graph is behind the `semantic` feature with the model runner.** `usearch` is C++ built from source; a consumer that reads repositories - `rigger` - neither builds nor links it.

## Consequences

Positive:

- Search by meaning answers from the first minute of the first reading, and says how much it does not cover yet.
- A renamed or moved file costs no reading: its passages are in the graph already, and only the chunks beside them change.
- A lost or damaged index costs seconds, never the hour of computing vectors.
- Opening the index costs a file read, not a pass over the full-text index.

Negative:

- The vectors are on disk twice - in the store and in the graph - which for thirty thousand chunks of a 384-dimensional model is about fifty megabytes each.
- An approximate answer can differ from the exact one. It is measured rather than argued: on the private archive of 13,260 passages and seventy questions, the graph and exact search scored the same (MRR 0.39), and differed on three questions only below the answer. When the two disagree on real questions, the graph's settings are what changes - the search width already did, from the default 64 to 256, after it missed one in ten over random vectors.
- Building the graph from scratch is not free: nine seconds for those 13,260 passages, minutes for tens of thousands of random vectors, the slowest case. A full build happens only when the index is lost or mostly rewritten; an update that changes nothing takes a fifth of a second.

Rejected alternatives:

- **Keep exact search.** Fast enough at today's sizes, and it leaves the two real problems - the cost of opening and the all-or-nothing answer - where they were.
- **Make the graph the only store.** One copy of each vector, and no record of what was computed that survives a damaged graph: the expensive thing would depend on the cheap one.
- **Key the graph by the store's record numbers.** No ids of its own, and every compaction of the store would force a rebuild.
- **Memory-map the index.** Less memory for a large library, and a Windows window holding the map would make every command-line update fail to replace the file.
