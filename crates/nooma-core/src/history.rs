//! Commit messages as documents.
//!
//! A repository's history is already a corpus of short documents about the
//! code, written by the people who changed it, explaining *why*. That "why" is
//! nowhere else: the code says what it does now, and only the commit that
//! introduced it says what it was for. "The commit where the PATH handling was
//! fixed" is a question the file index cannot answer at all and this one
//! answers directly.
//!
//! # Why this is cached differently
//!
//! A file is keyed by the hash of its contents because a file changes under a
//! stable name. A commit is the opposite: its id *is* the hash of everything
//! about it, so a commit already read can never need re-reading, and one that
//! is not held is simply new. There is no comparison pass and no invalidation.
//!
//! What the walk decides is *membership*, not just freshness. The commits
//! reachable from HEAD are the history; the stored one only says which of them
//! have already been read, and is consulted per commit rather than spliced on
//! at the point the walk stops. The difference shows up under `--amend` and
//! rebase: a rewritten commit is a different object, so the original stays in
//! the stored list, and a history that carried that list across would keep the
//! original alive beside its replacement — claiming a commit the repository no
//! longer has. Walking every reachable commit and reusing only what is still
//! reachable costs one pass over a list of hashes and makes that unrepresentable.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::repo::Repo;

/// The on-disk format of a stored history.
///
/// Held apart from the file index's version because the two are stored apart
/// and change for different reasons: adding a field to a commit entry has
/// nothing to do with how files are cut up.
pub const HISTORY_FORMAT_VERSION: u32 = 1;

/// One commit, as a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDoc {
    /// The full hex object id.
    pub id: String,
    /// The first line of the message.
    ///
    /// Kept apart from the body because it is the line people quote, the line
    /// a list shows, and — under this line's commit conventions — the one that
    /// names the kind of change.
    pub summary: String,
    /// The rest of the message, if there is any.
    pub body: Option<String>,
    /// Who wrote the change.
    pub author: String,
    /// When it was written, as seconds since the epoch.
    ///
    /// UTC, with the author's offset dropped: the offset says where someone
    /// was sitting, which no query here asks about, and keeping it would put
    /// two spellings of one instant in the index.
    pub time: i64,
    /// The paths this commit touched, relative to the repository root.
    ///
    /// This is what ties a message to the code it explains: "the commit where
    /// the PATH handling was fixed" is answered by matching the message, but
    /// "what was this file's last change for" is answered by matching the
    /// path.
    pub paths: Vec<String>,
}

impl CommitDoc {
    /// The commit as one block of text, for a reader or an embedder.
    ///
    /// The id is left out: a hash carries no meaning to match on, and it would
    /// be the longest run of characters in the document.
    pub fn to_text(&self) -> String {
        match &self.body {
            Some(body) => format!("{}\n\n{body}", self.summary),
            None => self.summary.clone(),
        }
    }

    /// The commit as people quote it.
    pub fn short(&self) -> &str {
        &self.id[..self.id.len().min(8)]
    }
}

/// Every commit reachable from one revision, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct History {
    /// The on-disk format, checked before anything else is believed.
    pub format_version: u32,
    /// The commit this history was read from.
    pub head: String,
    /// The absolute path of the work tree it belongs to.
    pub root: std::path::PathBuf,
    /// The commits, newest first.
    pub commits: Vec<CommitDoc>,
}

impl History {
    /// An empty history for a repository.
    pub fn empty(root: std::path::PathBuf, head: String) -> Self {
        Self {
            format_version: HISTORY_FORMAT_VERSION,
            head,
            root,
            commits: Vec::new(),
        }
    }

    /// Whether this history already describes the commit given.
    pub fn holds(&self, id: &str) -> bool {
        self.commits.iter().any(|commit| commit.id == id)
    }

    /// Commits whose message contains this text, compared without case.
    pub fn matching(&self, text: &str) -> Vec<&CommitDoc> {
        let needle = text.to_lowercase();
        self.commits.iter().filter(|commit| commit.to_text().to_lowercase().contains(&needle)).collect()
    }

    /// Commits that touched a path, or anything beneath it.
    pub fn touching(&self, prefix: &str) -> Vec<&CommitDoc> {
        self.commits
            .iter()
            .filter(|commit| commit.paths.iter().any(|path| path == prefix || path.starts_with(&format!("{prefix}/"))))
            .collect()
    }
}

/// What a history pass did, for the caller to report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HistoryUpdate {
    /// Commits read on this pass.
    pub read: usize,
    /// Commits carried across from the stored history.
    pub kept: usize,
}

impl HistoryUpdate {
    /// Whether the history came out of this pass as it went in.
    pub fn is_noop(&self) -> bool {
        self.read == 0
    }
}

/// Read a repository's history, reusing whatever was already read.
///
/// Walks every commit reachable from HEAD and reads only the ones the stored
/// history does not already hold. The walk decides membership as well as
/// freshness, so a commit the stored history holds but HEAD no longer reaches
/// — the original of an amend or a rebase — is dropped rather than kept.
///
/// `limit` caps how many commits are read on a single pass. A repository with
/// a hundred thousand commits should not make the first `index` appear to
/// hang, and the cap is the honest way to say so: what is read is the newest,
/// which is what every query here is about.
pub fn read(repo: &Repo, previous: Option<&History>, limit: usize) -> Result<(History, HistoryUpdate)> {
    let held: BTreeMap<&str, &CommitDoc> = previous
        .map(|history| history.commits.iter().map(|c| (c.id.as_str(), c)).collect())
        .unwrap_or_default();

    // The walk names the commits that are reachable *now*, and that list is
    // the history — the stored one only says which of them have already been
    // read. Carrying the stored list across instead, from the point the walk
    // stopped, would keep whatever it holds that the walk no longer reaches:
    // an amended or rebased commit is a different object, so the original
    // stays in the stored list and would survive beside its replacement,
    // leaving the history claiming a commit the repository does not have.
    let mut commits = Vec::new();
    let mut update = HistoryUpdate::default();
    for id in repo.commit_ids(limit)? {
        match held.get(id.as_str()) {
            Some(doc) => {
                commits.push((*doc).clone());
                update.kept += 1;
            }
            None => {
                commits.push(repo.commit_doc(&id)?);
                update.read += 1;
            }
        }
    }

    let mut history = History::empty(repo.root().to_path_buf(), repo.commit().to_string());
    history.commits = commits;
    Ok((history, update))
}

/// Where a repository's history is stored, beside its index.
pub fn path_for(store: &crate::store::Store, repo_root: &std::path::Path) -> std::path::PathBuf {
    store.path_for(repo_root).with_extension("history.json")
}

/// Read the stored history of a repository, if there is one.
pub fn load(store: &crate::store::Store, repo_root: &std::path::Path) -> Result<Option<History>> {
    let path = path_for(store, repo_root);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::io(&path, e)),
    };
    // The format is read before the rest, exactly as the file index does it: a
    // field that changed meaning must never be deserialized under the new
    // meaning and quietly believed.
    let probe: FormatProbe = serde_json::from_slice(&bytes).map_err(|source| Error::IndexUnreadable { path: path.clone(), source })?;
    if probe.format_version != HISTORY_FORMAT_VERSION {
        return Err(Error::IndexFormat {
            found: probe.format_version,
            expected: HISTORY_FORMAT_VERSION,
        });
    }
    let history: History = serde_json::from_slice(&bytes).map_err(|source| Error::IndexUnreadable { path, source })?;
    Ok(Some(history))
}

/// Write a history, replacing whatever was there.
pub fn save(store: &crate::store::Store, history: &History) -> Result<std::path::PathBuf> {
    let path = path_for(store, &history.root);
    let bytes = serde_json::to_vec(history).expect("a history is always serializable");
    // Written aside and renamed over, so a crash halfway leaves the previous
    // history intact rather than a truncated one.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, &bytes).map_err(|e| Error::io(&temporary, e))?;
    std::fs::rename(&temporary, &path).map_err(|e| Error::io(&path, e))?;
    Ok(path)
}

/// Just enough of the document to check the format before trusting the rest.
#[derive(serde::Deserialize)]
struct FormatProbe {
    format_version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: &str, summary: &str, paths: &[&str]) -> CommitDoc {
        CommitDoc {
            id: id.to_string(),
            summary: summary.to_string(),
            body: None,
            author: "A Writer".into(),
            time: 0,
            paths: paths.iter().map(|p| p.to_string()).collect(),
        }
    }

    fn sample() -> History {
        let mut history = History::empty("/repo".into(), "a".repeat(40));
        history.commits = vec![
            commit("a", "fix: correct the PATH handling on Windows", &["src/env.rs"]),
            commit("b", "feat: add the ledger", &["src/ledger.rs", "docs/ledger.md"]),
        ];
        history
    }

    #[test]
    fn a_message_is_matched_without_case() {
        let history = sample();
        assert_eq!(history.matching("path handling").len(), 1);
        assert_eq!(history.matching("PATH").len(), 1);
        assert_eq!(history.matching("nothing here").len(), 0);
    }

    /// The body is part of the document: the reason for a change is usually
    /// written below the summary, and a search over summaries alone would miss
    /// every explanation this index exists to find.
    #[test]
    fn the_body_is_searched_too() {
        let mut history = sample();
        history.commits[1].body = Some("Double entry from the first day.".into());
        assert_eq!(history.matching("double entry").len(), 1);
    }

    #[test]
    fn a_path_matches_itself_and_its_directory() {
        let history = sample();
        assert_eq!(history.touching("src/ledger.rs").len(), 1);
        assert_eq!(history.touching("src").len(), 2);
        // A prefix that is not a whole path component matches nothing: `src`
        // must not pull in `srcery/`.
        assert_eq!(history.touching("sr").len(), 0);
    }

    #[test]
    fn a_commit_reads_as_its_summary_and_body() {
        let mut one = commit("a", "feat: add the ledger", &[]);
        assert_eq!(one.to_text(), "feat: add the ledger");
        one.body = Some("Why it was added.".into());
        assert_eq!(one.to_text(), "feat: add the ledger\n\nWhy it was added.");
    }
}
