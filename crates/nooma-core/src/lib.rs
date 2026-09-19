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

pub mod error;
pub mod history;
pub mod incremental;
pub mod index;
pub mod lang;
pub mod repo;
pub mod store;
pub mod summary;
pub mod symbols;

pub use error::{Error, Result};
pub use history::{CommitDoc, History};
pub use incremental::Update;
pub use index::{FileIndex, Import, RepoIndex, Revision, Symbol, SymbolKind};
pub use lang::Language;
pub use repo::Repo;
pub use store::Store;
pub use summary::{ModuleSummary, SummaryEntry};
