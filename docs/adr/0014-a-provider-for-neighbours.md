# ADR 0014: A provider for neighbours, and commits ranked by meaning

- Status: accepted
- Date: 2026-11-25

## Context

Two programs of the line asked nooma for search they should not build
themselves.

scheda, an editor of Markdown vaults, keeps a structural index of its notes -
names, headings, links, tags - and searches their text with a grep. It wants
that text searched by words and by meaning, and asked for a trait to ask
through, so that its grep stays as the fallback.

furca, a git client, asks `nooma repo history --matching` for commits by a
word in the body of their message. It measured three things: a first pass
over a repository of five hundred commits took 24 seconds; `--matching` is a
substring, where its plan says "commits by meaning"; and `nooma repo` had no
environment variable for its store, so trying it out wrote into the real data
directory.

## Decision

**A trait, with nooma's own implementation beside it.** `provider::FullText`
is four methods - `update`, `search`, `explain`, `standing` - and
`provider::Folder` implements it over a `Library` with one source. It is a
trait rather than a struct because there are two answers to the question from
the first day: nooma's index, and the caller's own search of the files. The
caller holds a `dyn FullText` and falls back without a branch at every call.

**`Folder` adds nothing to the search.** It is `Library`, `Finder`,
`hybrid::search` and `why::explain`, held together and kept open. There is no
second ranking: the one list has one road (ADR 0007).

**One store per folder.** A store opened for another folder follows it. The
alternative - the folder as one more source in the user's own nooma library -
would put an editor's vault into what `nooma find` answers from without the
person having asked, and make two programs writers of one index.

**The model is attached by the caller, and never fetched.** `nooma-core` opens
no connection (ADR 0002), so a provider cannot get a model; it can use one
that is on disk. `with_model` takes the directory `nooma model fetch` writes
to, and returns the provider unchanged when the model is not there. Words
always answer.

**The update lends the model.** The first pass over a vault holds the model
for an hour. It takes it a batch at a time and waits while a search is
counted as wanting it - the mechanism the window has, now in the library
where a second caller needs it.

**Paths of a commit are read when asked for.** A message is one object; the
paths were two whole trees flattened and compared, per commit, and no search
of the messages reads them. `CommitDoc::paths` is `None` until
`History::read_paths` fills it: for every commit under `--touching`, for the
commits printed under `--json`, for none in a listing. `History::touching`
takes the repository and reads what is missing itself, so a commit never
compared cannot answer "did not touch it". The stored format is 2.

**The comparison opens only what differs.** ADR 0003's history read flattened
both trees, on the argument that a descent gone wrong would drop paths in
silence. That argument is answered by a test rather than by avoiding the
descent: every shape a change takes - a file three levels down, a directory
removed, a file become a directory and back, a move, a mode alone, a merge,
an empty commit - is held to what `git diff-tree -r --no-renames` reports.

**Commits are ranked exactly, by vectors kept under the hash of the message.**
`--meaning TEXT` orders the commits the filters left, by cosine with the
language taken out (ADR 0008), cut at `--top`. A history is thousands of short
texts, so every vector is compared and no graph is kept. The order asked for
vectors "by commit id"; the hash of the message is what the id fixes of it,
caches as permanently, and lets commits that say the same thing share a
vector.

**Without the model `--meaning` fails.** It does not fall back to the order of
the history: a list that looks ranked and is not would be believed.

**`NOOMA_REPO_STORE`, not `NOOMA_STORE`.** The library and the repository
indexes are different directories holding different things. One variable for
both would point one of them at the other's files.

## Consequences

- `HISTORY_FORMAT_VERSION` 1 → 2: a stored history is read again once, on the
  next `nooma repo history`. Nothing else is rebuilt.
- `--json` of `repo history` keeps its shape: `paths` is still an array on
  every row printed. A ranked list adds `score` to each row and `model` beside
  `head`.
- A commit that only changed a file's mode now names that file, as git does.
  Before, the comparison was of contents alone.
- The window still carries its own copy of the lending mechanism. Moving it
  onto `provider` is a refactoring of its own.
