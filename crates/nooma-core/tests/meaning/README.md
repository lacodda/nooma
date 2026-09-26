# Meaning-search test corpus

This corpus measures whether search finds a document by what it is about,
not by which words it happens to share with the query — across Russian and
English.

It has 8 everyday/technical topics (plumbing, sourdough starters, bicycle
chains, succulents, toddler sleep, phone photo backups, kettle descaling,
half-marathon training), each with 3 documents in `corpus/<topic>/`:

- **A (answer)** - answers one specific practical question.
- **B (near miss)** - same domain, a different question a weak model could
  confuse with A.
- **C (shared words)** - reuses A's surface vocabulary for an unrelated
  meaning (e.g. a tap vs. a construction crane, a chain vs. a blockchain).

Every text here is invented for this test suite: no real people, companies,
or notes.

`queries.json` holds 32 queries (4 per topic), each expecting its topic's
Doc A and grouped by `group`: `ru→ru` / `en→en` (same-language paraphrase
avoiding A's distinctive words), `ru→en` / `en→ru` (cross-language
paraphrases, two per topic), and `keywords` (a short same-language query
using A's key terms).
