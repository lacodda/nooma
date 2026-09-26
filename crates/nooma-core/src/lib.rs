//! The indexing library behind nooma.
//!
//! What this crate answers is one question: *what is in this repository, at
//! this commit?* It parses source files into symbols, imports and module
//! dependencies, and keeps the answer on disk so the next caller does not pay
//! for the parse again.
//!
//! It holds no UI and opens no network connection. `nooma` the application is
//! one caller; [`rigger`] is another, and was the reason this half was written
//! first.
//!
//! [`rigger`]: https://github.com/lacodda/rigger

pub mod document;
pub mod error;
pub mod fulltext;
pub mod history;
pub mod incremental;
pub mod index;
pub mod lang;
pub mod library;
pub mod repo;
pub mod store;
pub mod summary;
pub mod symbols;

pub use error::{Error, Result};

/// Where nooma keeps what it builds on this machine: the repository indexes,
/// the library and the models, each in its own folder below.
///
/// The machine's local data directory, not the roaming one. Everything here
/// is built from files on this machine or fetched for it - an index names
/// this machine's paths, a model is gigabytes - so none of it belongs in a
/// profile that follows the person to another computer.
pub fn data_dir() -> Result<std::path::PathBuf> {
    directories::ProjectDirs::from("com", "lacodda", "nooma")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .ok_or_else(|| Error::io(std::path::PathBuf::from("."), std::io::Error::other("no home directory on this system")))
}
pub use history::{CommitDoc, History};
pub use incremental::Update;
pub use index::{FileIndex, Import, RepoIndex, Revision, Symbol, SymbolKind};
pub use lang::Language;
pub use library::{Finder, Hit, Library, Progress, Skipped, Source, Status, UpdateReport};
pub use repo::Repo;
pub use store::Store;
pub use summary::{ModuleSummary, SummaryEntry};
