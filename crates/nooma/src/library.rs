//! `nooma source`, `nooma index` and `nooma find` — the document library on
//! the command line.
//!
//! What `--json` prints is a contract, as it is for `nooma repo`: `rigger`
//! and scripts read it, so fields are added, never renamed or dropped without
//! a format bump.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use nooma_core::{Error, Hit, Library, UpdateReport};

/// Where the library lives, for every command that opens it.
#[derive(Debug, Args)]
pub struct StoreArgs {
    /// Keep the library in this directory instead of the user's data directory
    #[arg(long, value_name = "DIR", global = true)]
    store: Option<PathBuf>,
}

impl StoreArgs {
    fn open(&self) -> Result<Library> {
        match &self.store {
            Some(dir) => Library::open_at(dir).with_context(|| format!("opening {}", dir.display())),
            None => Library::open().context("opening the library"),
        }
    }
}

/// The folders nooma searches.
#[derive(Debug, Args)]
pub struct SourceArgs {
    #[command(subcommand)]
    command: SourceCommand,
    #[command(flatten)]
    store: StoreArgs,
}

#[derive(Debug, Subcommand)]
enum SourceCommand {
    /// Search a folder; its documents are read at the next `nooma index`
    Add {
        /// The folder
        path: PathBuf,
        /// Leave out paths matching this pattern (.gitignore syntax); repeatable
        #[arg(long, value_name = "PATTERN")]
        exclude: Vec<String>,
    },
    /// Stop searching a folder; its documents leave at the next `nooma index`
    Remove {
        /// The folder, as it was added
        path: PathBuf,
    },
    /// List the folders and what the index holds
    List {
        /// Print machine-readable JSON instead of a report
        #[arg(long)]
        json: bool,
    },
}

/// Bring the index up to date with the folders.
#[derive(Debug, Args)]
pub struct IndexArgs {
    /// Print machine-readable JSON instead of a report
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    store: StoreArgs,
}

/// Find the documents that contain the words of a query.
#[derive(Debug, Args)]
pub struct FindArgs {
    /// What to look for; stemmed, so any form of a word finds the others
    query: String,
    /// Show at most this many documents
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// Print machine-readable JSON instead of a report
    #[arg(long)]
    json: bool,
    // As with `nooma repo`, a reading command brings the index up to date
    // first: the walk costs a stat per file, a wrong answer costs more.
    /// Answer from the stored index without bringing it up to date
    #[arg(long)]
    no_refresh: bool,
    #[command(flatten)]
    store: StoreArgs,
}

pub fn source(args: SourceArgs) -> Result<()> {
    let mut library = args.store.open()?;
    match args.command {
        SourceCommand::Add { path, exclude } => {
            let source = library.add_source(&path, exclude)?;
            println!("added {}", source.path.display());
            println!("run `nooma index` to read it");
        }
        SourceCommand::Remove { path } => {
            let source = library.remove_source(&path)?;
            println!("removed {}", source.path.display());
            println!("its documents leave the index at the next `nooma index`");
        }
        SourceCommand::List { json } => {
            let status = library.status()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
                return Ok(());
            }
            if status.sources.is_empty() {
                println!("no sources yet: `nooma source add <folder>`");
                return Ok(());
            }
            for source in &status.sources {
                println!("{}", source.path.display());
                for pattern in &source.exclude {
                    println!("  exclude {pattern}");
                }
            }
            println!();
            println!("{} documents, {} chunks", status.documents, status.chunks);
            match (&status.stale, status.indexed_at) {
                (Some(reason), _) => println!("the index must be rebuilt: {reason} — run `nooma index`"),
                (None, None) => println!("not indexed yet — run `nooma index`"),
                (None, Some(_)) => {}
            }
        }
    }
    Ok(())
}

pub fn index(args: IndexArgs) -> Result<()> {
    let library = args.store.open()?;
    if library.sources().is_empty() {
        anyhow::bail!("no sources to index: `nooma source add <folder>` first");
    }
    let report = library.update()?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }
    Ok(())
}

fn print_report(report: &UpdateReport) {
    if let Some(reason) = &report.rebuilt {
        println!("rebuilt the index from scratch: {reason}");
    }
    println!(
        "{} documents · {} read · {} removed · {} chunks written · {} ms",
        report.documents, report.indexed, report.removed, report.chunks, report.took_ms
    );
    for source in &report.unavailable {
        println!("unavailable, kept as it was: {}", source.display());
    }
    for skipped in &report.skipped {
        println!("skipped {}: {}", skipped.path.display(), skipped.reason);
    }
}

pub fn find(args: FindArgs) -> Result<()> {
    let library = args.store.open()?;
    let mut refreshed = false;
    if !args.no_refresh && !library.sources().is_empty() {
        match library.update() {
            Ok(_) => refreshed = true,
            // Another nooma is writing the index right now. The stored index
            // is still a correct answer about the moment it was written, so it
            // is used — and the output says it was not refreshed.
            Err(Error::Busy) => eprintln!("nooma: another nooma is indexing; answering from the stored index"),
            Err(error) => return Err(error.into()),
        }
    }
    let hits = library.search(&args.query, args.limit)?;
    let status = library.status()?;

    if args.json {
        let out = serde_json::json!({
            "query": args.query,
            "refreshed": refreshed,
            "indexed_at": status.indexed_at,
            "hits": hits,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if status.sources.is_empty() {
        println!("no sources yet: `nooma source add <folder>`, then search");
        return Ok(());
    }
    if hits.is_empty() {
        println!("nothing found for {:?}", args.query);
        return Ok(());
    }
    for hit in &hits {
        print_hit(hit);
    }
    Ok(())
}

fn print_hit(hit: &Hit) {
    let mut heading = hit.title.clone();
    if !hit.headings.is_empty() {
        heading.push_str(" · ");
        heading.push_str(&hit.headings.join(" › "));
    }
    println!("{heading}");
    println!("  {}:{}", hit.path.display(), hit.line);
    let fragment = hit.fragment.split_whitespace().collect::<Vec<_>>().join(" ");
    if !fragment.is_empty() {
        println!("  {fragment}");
    }
    println!();
}
