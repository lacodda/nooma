//! `nooma source`, `nooma index`, `nooma find` and `nooma similar` — the
//! document library on the command line.
//!
//! What `--json` prints is a contract, as it is for `nooma repo`: `rigger`
//! and scripts read it, so fields are added, never renamed or dropped without
//! a format bump.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use nooma_core::model::ModelSpec;
use nooma_core::{Error, Hit, Library, Role, UpdateReport, VectorReport};

use crate::meaning::{self, Meaning, State};
use crate::model::ModelsArgs;

/// Where the library lives, for every command that opens it.
#[derive(Debug, Args)]
pub struct StoreArgs {
    /// Keep the library in this directory instead of the user's data directory
    #[arg(long, value_name = "DIR", global = true, env = "NOOMA_STORE")]
    store: Option<PathBuf>,
}

impl StoreArgs {
    pub(crate) fn open(&self) -> Result<Library> {
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
    #[command(flatten)]
    models: ModelsArgs,
}

/// Find documents by the words of a query, and by what it means.
#[derive(Debug, Args)]
pub struct FindArgs {
    /// What to look for; stemmed, so any form of a word finds the others
    query: String,
    /// Show at most this many documents in each list
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
    #[command(flatten)]
    models: ModelsArgs,
}

/// Find documents that say what a passage says.
#[derive(Debug, Args)]
pub struct SimilarArgs {
    /// A file holding the passage, or `-` to read it from standard input
    example: PathBuf,
    /// Show at most this many documents
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// Print machine-readable JSON instead of a report
    #[arg(long)]
    json: bool,
    /// Answer from the stored index without bringing it up to date
    #[arg(long)]
    no_refresh: bool,
    #[command(flatten)]
    store: StoreArgs,
    #[command(flatten)]
    models: ModelsArgs,
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
        bail!("no sources to index: `nooma source add <folder>` first");
    }
    let report = library.update()?;
    if !args.json {
        print_report(&report);
    }
    let vectors = update_vectors(&library, &args.models.dir()?)?;
    if args.json {
        let mut out = serde_json::to_value(&report)?;
        out["vectors"] = match &vectors {
            Vectors::Updated(report) => serde_json::to_value(report)?,
            Vectors::NoModel | Vectors::Busy => serde_json::Value::Null,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    match &vectors {
        Vectors::Updated(report) => print_vectors(report),
        Vectors::NoModel => {
            let spec = ModelSpec::default_model();
            println!(
                "search by meaning needs its model: `nooma model fetch` ({}, the one download nooma makes)",
                crate::model::size(spec.bytes())
            );
        }
        Vectors::Busy => println!("another nooma is computing the vectors of this library; they are left to it"),
    }
    Ok(())
}

/// What became of the vectors in an update.
enum Vectors {
    Updated(VectorReport),
    /// The model has not been fetched; there is nothing to compute them with.
    NoModel,
    /// Another process - the window, most likely - is computing them.
    Busy,
}

fn update_vectors(library: &Library, models: &Path) -> Result<Vectors> {
    let Some(mut embedder) = meaning::load(models)? else {
        return Ok(Vectors::NoModel);
    };
    match library.update_vectors(&mut embedder, &meaning::progress_for(ModelSpec::default_model().id)) {
        Ok(report) => Ok(Vectors::Updated(report)),
        Err(Error::Busy) => Ok(Vectors::Busy),
        Err(error) => Err(error.into()),
    }
}

fn print_vectors(report: &VectorReport) {
    if let Some(reason) = &report.rebuilt {
        println!("started the vectors again: {reason}");
    }
    if report.embedded == 0 {
        println!("{}: every one of {} passages has its vector", report.model, report.passages);
        return;
    }
    let per_second = report.embedded as f64 / (report.embed_ms.max(1) as f64 / 1000.0);
    println!(
        "{}: computed {} of {} passages in {:.0} s ({per_second:.1} a second)",
        report.model,
        report.embedded,
        report.passages,
        report.embed_ms as f64 / 1000.0
    );
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

/// Bring the full-text index up to date before a reading command answers;
/// returns whether it was.
pub(crate) fn refresh(library: &Library) -> Result<bool> {
    if library.sources().is_empty() {
        return Ok(false);
    }
    match library.update() {
        Ok(_) => Ok(true),
        // Another nooma is writing the index right now. The stored index is
        // still a correct answer about the moment it was written, so it is
        // used — and the output says it was not refreshed.
        Err(Error::Busy) => {
            eprintln!("nooma: another nooma is indexing; answering from the stored index");
            Ok(false)
        }
        Err(error) => Err(error.into()),
    }
}

pub fn find(args: FindArgs) -> Result<()> {
    let library = args.store.open()?;
    let refreshed = !args.no_refresh && refresh(&library)?;
    let finder = library.finder()?;
    let hits = match &finder {
        Some(finder) => finder.search(&args.query, args.limit)?,
        None => Vec::new(),
    };
    let meaning = meaning::search(&library, finder.as_ref(), &args.models.dir()?, &args.query, Role::Query, args.limit, None)?;
    let status = library.status()?;

    if args.json {
        let out = serde_json::json!({
            "query": args.query,
            "refreshed": refreshed,
            "indexed_at": status.indexed_at,
            "hits": hits,
            "meaning": meaning.to_json(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if status.sources.is_empty() {
        println!("no sources yet: `nooma source add <folder>`, then search");
        return Ok(());
    }
    println!("exact");
    println!();
    if hits.is_empty() {
        println!("  nothing contains the words of {:?}", args.query);
        println!();
    }
    for hit in &hits {
        print_hit(hit, false);
    }
    print_meaning(&meaning, &format!("nothing found by meaning for {:?}", args.query));
    Ok(())
}

pub fn similar(args: SimilarArgs) -> Result<()> {
    let (text, example) = if args.example.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).context("reading the passage from standard input")?;
        (text, None)
    } else {
        let text = std::fs::read_to_string(&args.example).with_context(|| format!("reading {}", args.example.display()))?;
        (text, Some(std::path::absolute(&args.example)?))
    };
    if text.trim().is_empty() {
        bail!("the passage is empty: there is nothing to find more of");
    }
    let library = args.store.open()?;
    if library.sources().is_empty() {
        bail!("no sources yet: `nooma source add <folder>`, then `nooma index`");
    }
    if !args.no_refresh {
        refresh(&library)?;
    }
    let finder = library.finder()?;
    // A passage is compared with passages: it is read as one, not as a
    // question.
    let meaning = meaning::search(
        &library,
        finder.as_ref(),
        &args.models.dir()?,
        &text,
        Role::Passage,
        args.limit,
        example.as_deref(),
    )?;
    match meaning.state {
        State::Ready => {}
        State::NoModel => bail!(
            "search by example needs the model: `nooma model fetch` ({})",
            crate::model::size(ModelSpec::default_model().bytes())
        ),
        State::NoVectors => bail!("no vectors yet: `nooma index` computes them"),
    }

    if args.json {
        let mut out = meaning.to_json();
        out["example"] = serde_json::json!(args.example);
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    print_meaning(&meaning, "nothing in the library is close to this passage");
    Ok(())
}

/// The by-meaning half of a report: its hits, and what it could not cover.
fn print_meaning(meaning: &Meaning, nothing: &str) {
    if meaning.state == State::Ready {
        println!("by meaning · {}", meaning.model);
        println!();
        if meaning.hits.is_empty() {
            println!("  {nothing}");
            println!();
        }
        for hit in &meaning.hits {
            print_hit(hit, true);
        }
    }
    if let Some(caveat) = meaning::caveat(meaning) {
        println!("{caveat}");
    }
}

fn print_hit(hit: &Hit, scored: bool) {
    let mut heading = hit.title.clone();
    // A note's top heading is usually its title; saying it twice is noise.
    let headings = match hit.headings.split_first() {
        Some((first, rest)) if *first == hit.title => rest,
        _ => &hit.headings[..],
    };
    if !headings.is_empty() {
        heading.push_str(" · ");
        heading.push_str(&headings.join(" › "));
    }
    if scored {
        // A cosine, comparable across searches: how close, not just which
        // is closer.
        println!("{heading} ({:.2})", hit.score);
    } else {
        println!("{heading}");
    }
    println!("  {}:{}", hit.path.display(), hit.line);
    let fragment = hit.fragment.split_whitespace().collect::<Vec<_>>().join(" ");
    if !fragment.is_empty() {
        println!("  {fragment}");
    }
    println!();
}
