//! nooma — local semantic search over your files.
//!
//! Two indexes are queried together: `tantivy` for exact matches and a vector
//! index for meaning. Neither half alone is the product — see
//! `docs/adr/0001-hybrid-index.md`.
//!
//! `nooma find` searches the folders added with `nooma source add`, through
//! the exact half; `nooma model` fetches the embedding model the other half
//! runs, and `nooma eval` measures both against questions with known answers;
//! `nooma repo` reads a git work tree into symbols, imports and module
//! dependencies. The window is a separate binary, `nooma-app`, over the same
//! library.

mod eval;
mod library;
mod model;
#[cfg(feature = "prose")]
mod prose;
mod repo;

use clap::{Parser, Subcommand};

/// Local search that finds a file by what it is about.
#[derive(Debug, Parser)]
#[command(name = "nooma", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Find the documents that contain the words of a query
    Find(library::FindArgs),
    /// Bring the index up to date with the folders
    Index(library::IndexArgs),
    /// Add, remove and list the folders nooma searches
    Source(library::SourceArgs),
    /// List, fetch and remove the embedding models
    Model(model::ModelArgs),
    /// Measure search against questions whose answers are known
    Eval(eval::EvalArgs),
    /// What is in a repository: symbols, imports and module dependencies
    Repo(repo::RepoArgs),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Find(args) => library::find(args),
        Command::Index(args) => library::index(args),
        Command::Source(args) => library::source(args),
        Command::Model(args) => model::run(args),
        Command::Eval(args) => eval::run(args),
        Command::Repo(args) => repo::run(args),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        // The chain is printed, not just the outermost message: "the stored
        // index could not be read" without the parse error underneath tells
        // nobody which byte was wrong.
        Err(error) => {
            eprintln!("nooma: {error}");
            for cause in error.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
