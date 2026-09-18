//! Reindexing only what changed.
//!
//! The rule is one sentence: a file whose bytes hash to what the index already
//! recorded is not reparsed. That covers both questions the stage was asked -
//! what changed between two commits, and what changed in the working tree -
//! with one mechanism, because both reduce to the same comparison.
//!
//! Git's own diff is deliberately not consulted. It would be a second source
//! of truth about what changed, and the two disagree exactly where it hurts:
//! a file written after git last looked is changed by every measure that
//! matters here and by none that git reports yet. Trusting git there would
//! leave the newest edit unindexed, which reads as search having quietly
//! stopped working.
//!
//! What this saves is parsing, not reading. Hashing a megabyte costs
//! milliseconds; parsing a repository the size of `dowel` costs two seconds.
//! So every file is read and hashed on every pass, and only the mismatches go
//! to tree-sitter.

use rayon::prelude::*;

use crate::error::Result;
use crate::index::{FileIndex, RepoIndex, Revision};
use crate::repo::{Repo, SourceFile};
use crate::symbols;

/// A file as it was found on disk: its hash, and the bytes that produced it.
///
/// The bytes are carried rather than read a second time: the file has to be
/// read to be hashed at all, and a second read could land on different
/// content, which would file the new bytes under the old hash.
type Read = (String, Vec<u8>);

/// What an incremental pass did, for the caller to report.
///
/// Counted rather than derived afterwards: `added + changed + unchanged` is
/// the number of files now in the index, and `removed` is the only way to tell
/// a deletion from a file that was never there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Update {
    /// Files in the tree that the index had never seen.
    pub added: usize,
    /// Files whose bytes no longer hash to what was recorded.
    pub changed: usize,
    /// Files whose entries were reused untouched.
    pub unchanged: usize,
    /// Files the index held that are no longer in the tree.
    pub removed: usize,
}

impl Update {
    /// How many files had to be parsed.
    pub fn parsed(&self) -> usize {
        self.added + self.changed
    }

    /// Whether the index came out of this pass the same as it went in.
    pub fn is_noop(&self) -> bool {
        self.added == 0 && self.changed == 0 && self.removed == 0
    }
}

/// Bring an index up to date with the work tree, reparsing only what changed.
///
/// `previous` is the stored index, if there is one that can be reused. Passing
/// `None` indexes everything, which is what a first run and a forced rebuild
/// both want - so there is one code path, not a fast one and a slow one that
/// drift apart.
pub fn update(repo: &Repo, previous: Option<&RepoIndex>) -> Result<(RepoIndex, Update)> {
    let files = repo.source_files()?;
    let reusable = previous.filter(|index| index.is_reusable());
    plan_and_parse(repo, &files, reusable)
}

/// The pass itself, once the file list is in hand.
fn plan_and_parse(repo: &Repo, files: &[SourceFile], previous: Option<&RepoIndex>) -> Result<(RepoIndex, Update)> {
    let held = previous.map(RepoIndex::by_path).unwrap_or_default();

    // Read and hash every file first, in parallel. This is the pass that
    // decides what to parse, so it cannot itself be skipped - and it is cheap
    // next to parsing.
    let hashed: Vec<(usize, Option<Read>)> = files
        .par_iter()
        .enumerate()
        .map(|(position, file)| {
            let read = std::fs::read(&file.absolute)
                .ok()
                .map(|bytes| (blake3::hash(&bytes).to_hex().to_string(), bytes));
            (position, read)
        })
        .collect();

    // Decide, serially and cheaply, which of them need the parser. Reusing an
    // entry means moving it across unchanged, not reparsing to the same
    // answer.
    let mut update = Update::default();
    let mut reused: Vec<FileIndex> = Vec::new();
    let mut to_parse: Vec<(&SourceFile, String, Vec<u8>)> = Vec::new();
    let mut seen_paths: Vec<&str> = Vec::with_capacity(files.len());

    for (position, read) in hashed {
        let file = &files[position];
        // A file that cannot be read is not in the tree as far as this pass is
        // concerned: it is left out rather than carried over stale.
        let Some((content_hash, bytes)) = read else {
            continue;
        };
        seen_paths.push(file.relative.as_str());
        match held.get(file.relative.as_str()) {
            Some(entry) if entry.content_hash == content_hash => {
                update.unchanged += 1;
                reused.push((*entry).clone());
            }
            Some(_) => {
                update.changed += 1;
                to_parse.push((file, content_hash, bytes));
            }
            None => {
                update.added += 1;
                to_parse.push((file, content_hash, bytes));
            }
        }
    }

    // Anything the index held that the walk did not meet has left the tree.
    update.removed = held.keys().filter(|path| !seen_paths.contains(path)).count();

    // Only now, the expensive part, and only on what actually changed.
    let parsed: Vec<FileIndex> = to_parse
        .into_par_iter()
        .filter_map(|(file, content_hash, bytes)| {
            let (symbols, imports) = symbols::parse(file.language, &bytes)?;
            Some(FileIndex {
                path: file.relative.clone(),
                language: file.language,
                content_hash,
                symbols,
                imports,
            })
        })
        .collect();

    let mut all = reused;
    all.extend(parsed);
    // Sorted, so that an index of one revision is the same bytes every run
    // whatever order the work happened to finish in - which is what lets two
    // indexes be compared at all.
    all.sort_by(|a, b| a.path.cmp(&b.path));

    let revision = revision_of(repo, &all);
    let mut index = RepoIndex::empty(repo.root().to_path_buf(), revision);
    index.files = all;
    Ok((index, update))
}

/// What revision the indexed files amount to.
///
/// The commit comes from git; `dirty` comes from comparing what was just
/// hashed against what that commit holds. Asking git for the answer would
/// report a file written since git last looked as clean.
fn revision_of(repo: &Repo, files: &[FileIndex]) -> Revision {
    // Both directions have to be checked, and a deleted file is only visible
    // in one of them. Asking the commit about the files the tree has would
    // never mention a file the tree has lost, so the commit is asked for its
    // whole list of indexable files and the two are compared as sets.
    let dirty = match repo.committed_source_hashes() {
        Ok(committed) => committed.len() != files.len() || files.iter().any(|file| committed.get(&file.path) != Some(&file.content_hash)),
        // The commit could not be read for comparison. Saying "clean" would
        // claim something unverified; "dirty" only costs one reparse later.
        Err(_) => true,
    };
    Revision {
        commit: repo.commit().to_string(),
        dirty,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_that_touched_nothing_is_a_noop() {
        let quiet = Update {
            added: 0,
            changed: 0,
            unchanged: 12,
            removed: 0,
        };
        assert!(quiet.is_noop());
        assert_eq!(quiet.parsed(), 0);
    }

    #[test]
    fn a_removal_alone_is_not_a_noop() {
        // Nothing was parsed, but the index did change: a caller that treats
        // "parsed nothing" as "nothing happened" would skip the write.
        let deletion = Update {
            added: 0,
            changed: 0,
            unchanged: 12,
            removed: 1,
        };
        assert!(!deletion.is_noop());
        assert_eq!(deletion.parsed(), 0);
    }

    #[test]
    fn parsed_counts_both_kinds_of_work() {
        let mixed = Update {
            added: 2,
            changed: 3,
            unchanged: 40,
            removed: 1,
        };
        assert_eq!(mixed.parsed(), 5);
        assert!(!mixed.is_noop());
    }
}
