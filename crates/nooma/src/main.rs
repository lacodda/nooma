//! nooma — local semantic search over your files.
//!
//! Two indexes are queried together: `tantivy` for exact matches and a vector
//! index for meaning. Neither half alone is the product — see
//! `docs/adr/0001-hybrid-index.md`.
//!
//! This version ships the repository half: `nooma repo` reads a git work tree
//! into symbols, imports and module dependencies. Document search arrives with
//! the index halves it needs.

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
    /// What is in a repository: symbols, imports and module dependencies
    Repo(repo::RepoArgs),
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
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
