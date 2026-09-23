//! What can go wrong, said precisely enough to act on.
//!
//! Callers of this crate are other programs, not people reading a terminal:
//! `rigger` needs to tell "this is not a repository" from "the index on disk
//! was written by an older nooma" without matching on message text.

use std::path::PathBuf;

/// The result of anything this crate does.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything `nooma-core` can fail at.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The path is not inside a git work tree.
    #[error("{0} is not inside a git repository")]
    NotARepository(PathBuf),

    /// The repository has no commits yet, so there is nothing to index.
    #[error("the repository at {0} has no commits yet")]
    EmptyRepository(PathBuf),

    /// A stored index was written by a different index format.
    ///
    /// Held apart from a plain read error on purpose: this one is recoverable
    /// by reindexing, and the caller is expected to do exactly that rather
    /// than report a corrupt file.
    #[error("the stored index is format {found}, this nooma writes {expected} — reindex")]
    IndexFormat {
        /// The `format_version` read from disk.
        found: u32,
        /// The `format_version` this build writes.
        expected: u32,
    },

    /// The index on disk could not be read as an index at all.
    #[error("the stored index at {path} could not be read: {source}")]
    IndexUnreadable {
        /// Where the unreadable file sits.
        path: PathBuf,
        /// What the deserializer said.
        source: serde_json::Error,
    },

    /// A tree-sitter grammar refused the source, or the parser was misbuilt.
    #[error("failed to parse {path} as {language}")]
    Parse {
        /// The file that would not parse.
        path: PathBuf,
        /// The language the grammar was chosen for.
        language: crate::lang::Language,
    },

    /// Anything git refused that does not deserve a case of its own.
    ///
    /// Reading a commit's own objects can fail in ways a caller cannot act on
    /// differently - a corrupt pack, a missing object - so they share one
    /// case rather than each getting a name nobody matches on.
    #[error("git: {0}")]
    Git(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// The full-text index refused an operation.
    #[error("full-text index: {0}")]
    Fulltext(#[source] Box<tantivy::TantivyError>),

    /// Another process is updating the same library right now.
    #[error("another nooma is updating this library; try again when it has finished")]
    Busy,

    /// The stored library index was built by other rules and has to be built
    /// again before it can be searched.
    #[error("the stored index cannot be searched: {0} — reindex")]
    LibraryStale(String),

    /// A source has to be a folder.
    #[error("{0} is not a folder")]
    NotADirectory(PathBuf),

    /// A source would overlap one the library already has.
    #[error("{path} overlaps the source {existing}; a file may belong to one source only")]
    SourceOverlap {
        /// The folder that was being added.
        path: PathBuf,
        /// The source it overlaps.
        existing: PathBuf,
    },

    /// There is no such source to remove.
    #[error("{0} is not a source of this library")]
    UnknownSource(PathBuf),

    /// An exclusion pattern is not valid `.gitignore` syntax.
    #[error("the pattern {0:?} is not valid: {1}")]
    BadPattern(String, String),

    /// Anything the filesystem refused.
    #[error("{path}: {source}")]
    Io {
        /// The path that was being read or written.
        path: PathBuf,
        /// The underlying failure.
        source: std::io::Error,
    },
}

impl Error {
    /// Wrap a git failure whose type is not worth naming in the signature.
    pub(crate) fn git(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Git(Box::new(source))
    }

    /// Wrap a full-text index failure.
    pub(crate) fn fulltext(source: tantivy::TantivyError) -> Self {
        Self::Fulltext(Box::new(source))
    }

    /// Attach a path to an [`std::io::Error`], which never carries one.
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }

    /// Whether reindexing from scratch would clear this.
    ///
    /// `rigger` asks the library rather than guessing from the message: a
    /// stale format is a normal event after an upgrade, an unreadable file is
    /// a normal event after a crash mid-write, and both are fixed the same
    /// way.
    pub fn is_stale_index(&self) -> bool {
        matches!(self, Self::IndexFormat { .. } | Self::IndexUnreadable { .. } | Self::LibraryStale(_))
    }
}
