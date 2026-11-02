//! `nooma source`, `nooma index`, `nooma health`, `nooma find` and `nooma
//! similar` — the document library on the command line.
//!
//! What `--json` prints is a contract, as it is for `nooma repo`: `rigger`
//! and scripts read it, so fields are added, never renamed or dropped without
//! a format bump.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{Context as _, Result, anyhow, bail};
use clap::{Args, Subcommand};
use nooma_core::document::DocKind;
use nooma_core::embed::OnnxEmbedder;
use nooma_core::model::ModelSpec;
use nooma_core::reading::Unread;
use nooma_core::{Error, Health, Library, Pace, Ranked, UpdateReport, VectorReport, Why, background};

use crate::meaning::{self, State};
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
    /// Print machine-readable JSON instead of a report; with --watch, one
    /// line of it for every update
    #[arg(long)]
    json: bool,
    /// Then keep the index up to date as files change, until stopped:
    /// below other programs, and waiting while the machine is on battery
    #[arg(long)]
    watch: bool,
    #[command(flatten)]
    store: StoreArgs,
    #[command(flatten)]
    models: ModelsArgs,
}

/// Say what the index could not read, and why.
#[derive(Debug, Args)]
pub struct HealthArgs {
    /// Print machine-readable JSON instead of a report
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    store: StoreArgs,
}

/// Find documents by the words of a query, and by what it means.
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
        print_standing(&library, &report)?;
    }
    let mut embedder = meaning::load(&args.models.dir()?)?;
    let vectors = update_vectors(
        &library,
        embedder.as_mut(),
        Pace::foreground(&meaning::progress_for(ModelSpec::default_model().id)),
    )?;
    if args.json {
        let out = json_of(&report, &vectors)?;
        match args.watch {
            true => println!("{}", serde_json::to_string(&out)?),
            false => println!("{}", serde_json::to_string_pretty(&out)?),
        }
    } else {
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
    }
    if args.watch {
        keep_up(&library, embedder.as_mut(), args.json)?;
    }
    Ok(())
}

/// An update's report with its vectors', as `--json` prints it.
fn json_of(report: &UpdateReport, vectors: &Vectors) -> Result<serde_json::Value> {
    let mut out = serde_json::to_value(report)?;
    out["vectors"] = match vectors {
        Vectors::Updated(report) => serde_json::to_value(report)?,
        Vectors::NoModel | Vectors::Busy => serde_json::Value::Null,
    };
    Ok(out)
}

/// Keep the index up to date with the folders as files change, until the
/// process is stopped: the work nobody waits for, run as the window runs it -
/// below other programs, holding while the machine is on its battery.
fn keep_up(library: &Library, mut embedder: Option<&mut OnnxEmbedder>, json: bool) -> Result<()> {
    background::lower_this_thread();
    let (told, heard) = mpsc::channel();
    let watcher = library.watch(move |scope| {
        // The receiver goes only when the process does.
        let _ = told.send(scope);
    })?;
    for (path, why) in watcher.unwatched() {
        eprintln!("nooma: not watching {}: {why}", path.display());
    }
    eprintln!(
        "nooma: watching {} for changes; Ctrl+C stops",
        if library.sources().len() == 1 {
            "the folder".to_string()
        } else {
            format!("{} folders", library.sources().len())
        }
    );
    let held = || eprintln!("nooma: on battery - indexing waits for the power");
    let hold = || background::hold(&held);
    let quiet = |_| {};
    while let Ok(mut scope) = heard.recv() {
        // What changed while the last update ran is one update.
        while let Ok(more) = heard.try_recv() {
            scope = scope.and(more);
        }
        let report = match library.update_scoped(&scope, Pace::background(&quiet, &hold)) {
            Ok(report) => report,
            Err(Error::Busy) => {
                eprintln!("nooma: another nooma is indexing; its update covers this change");
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        let changed = report.indexed > 0 || report.removed > 0 || !report.skipped.is_empty() || report.rebuilt.is_some();
        // The vectors follow the chunks, and the chunks moved only if a
        // document was read or removed.
        let vectors = match report.indexed > 0 || report.removed > 0 {
            true => update_vectors(library, embedder.as_deref_mut(), Pace::background(&quiet, &hold))?,
            false => Vectors::NoModel,
        };
        let embedded = matches!(&vectors, Vectors::Updated(vectors) if vectors.embedded > 0);
        if !changed && !embedded {
            continue;
        }
        if json {
            println!("{}", serde_json::to_string(&json_of(&report, &vectors)?)?);
            continue;
        }
        print_report(&report);
        if let Vectors::Updated(vectors) = &vectors
            && vectors.embedded > 0
        {
            print_vectors(vectors);
        }
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

fn update_vectors(library: &Library, embedder: Option<&mut OnnxEmbedder>, pace: Pace<'_>) -> Result<Vectors> {
    let Some(embedder) = embedder else {
        return Ok(Vectors::NoModel);
    };
    match library.update_vectors_paced(embedder, pace) {
        Ok(report) => Ok(Vectors::Updated(report)),
        Err(Error::Busy) => Ok(Vectors::Busy),
        Err(error) => Err(error.into()),
    }
}

/// The files the index lacks beyond those this update named: a file refused
/// once is not tried again until it changes, and is said here so it is not
/// forgotten.
fn print_standing(library: &Library, report: &UpdateReport) -> Result<()> {
    let standing = library.health()?.skipped.len();
    if standing > report.skipped.len() {
        let files = if standing == 1 { "file is" } else { "files are" };
        println!("{standing} {files} not in the index: `nooma health` says which and why");
    }
    Ok(())
}

pub fn health(args: HealthArgs) -> Result<()> {
    let library = args.store.open()?;
    let health = library.health()?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&health)?);
        return Ok(());
    }
    if library.sources().is_empty() {
        println!("no sources yet: `nooma source add <folder>`");
        return Ok(());
    }
    print_health(&health);
    Ok(())
}

/// What a reason is called, heading the files not read for it.
fn why_heading(why: Why) -> &'static str {
    match why {
        Why::NotADocument => "not documents nooma reads",
        Why::TooLarge => "too large",
        Why::Locked => "under a password or DRM",
        Why::Unopened => "could not be opened - tried again at every update",
        Why::Damaged => "damaged, or not readable by their reader",
    }
}

fn print_health(health: &Health) {
    if health.count() == 0 {
        println!("every file found was read");
        return;
    }
    if !health.skipped.is_empty() {
        println!("not read: {}", health.skipped.len());
        let mut last = None;
        for skipped in &health.skipped {
            if last != Some(skipped.why) {
                println!("  {}", why_heading(skipped.why));
                last = Some(skipped.why);
            }
            let size = match (skipped.why, skipped.size) {
                // In the megabytes the limit is said in.
                (Why::TooLarge, Some(size)) => format!(" ({:.0} MB)", size as f64 / 1024.0 / 1024.0),
                _ => String::new(),
            };
            println!("    {} - {}{size}", skipped.path.display(), skipped.reason);
        }
    }
    if !health.textless.is_empty() {
        println!("found by their names only, with no text in them: {}", health.textless.len());
        if let Some(why) = &health.no_ocr {
            println!("  text in scans and pictures is not read: {why}");
        }
        for path in &health.textless {
            println!("    {}", path.display());
        }
    }
    if !health.unrecognized.is_empty() {
        println!("read without their pictures of text: {}", health.unrecognized.len());
        for unrecognized in &health.unrecognized {
            println!("    {} - {}", unrecognized.path.display(), unrecognized.reason);
        }
    }
    if !health.guessed.is_empty() {
        println!("read in a guessed encoding: {}", health.guessed.len());
        for guessed in &health.guessed {
            println!("    {} - {}", guessed.path.display(), guessed.encoding);
        }
    }
    if !health.unavailable.is_empty() {
        println!("folders out of reach, their documents kept as they were: {}", health.unavailable.len());
        for path in &health.unavailable {
            println!("    {}", path.display());
        }
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
    for unrecognized in &report.unrecognized {
        println!("read without its pictures of text {}: {}", unrecognized.path.display(), unrecognized.reason);
    }
    for path in &report.textless {
        println!("no text in {} - a scan? found by its name only", path.display());
    }
    // Scans that stay nameless for want of OCR say why, once.
    if !report.textless.is_empty()
        && let Err(why) = nooma_core::ocr::ocr()
    {
        println!("text in scans and pictures is not read: {why}");
    }
    if !report.recognized.is_empty() {
        let files = if report.recognized.len() == 1 { "file" } else { "files" };
        println!("{} {files} read by OCR, in whole or in part", report.recognized.len());
    }
    print_guessed(report);
}

/// The files whose encoding was guessed, counted by the guess: a wrong one is
/// the reason a file is found by nothing, and the list in `--json` names
/// them.
fn print_guessed(report: &UpdateReport) {
    if report.guessed.is_empty() {
        return;
    }
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for guessed in &report.guessed {
        match counts.iter_mut().find(|(encoding, _)| *encoding == guessed.encoding) {
            Some((_, count)) => *count += 1,
            None => counts.push((&guessed.encoding, 1)),
        }
    }
    let counted: Vec<String> = counts.iter().map(|(encoding, count)| format!("{encoding} {count}")).collect();
    let files = if report.guessed.len() == 1 { "file is" } else { "files are" };
    println!("{} {files} not UTF-8, read in a guessed encoding: {}", report.guessed.len(), counted.join(", "));
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
    let (answer, meaning) = meaning::search(&library, finder.as_ref(), &args.models.dir()?, &args.query, args.limit)?;
    let status = library.status()?;

    if args.json {
        let out = serde_json::json!({
            "query": args.query,
            "refreshed": refreshed,
            "indexed_at": status.indexed_at,
            "all_words": answer.all_words,
            "hits": answer.hits,
            "meaning": meaning.to_json(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if status.sources.is_empty() {
        println!("no sources yet: `nooma source add <folder>`, then search");
        return Ok(());
    }
    if answer.hits.is_empty() {
        println!("nothing found for {:?}", args.query);
        println!();
    } else if !answer.all_words && answer.hits.iter().any(|ranked| ranked.words.is_some()) {
        // Said once, above the list: a row marked by its words holds some of
        // them, not every one.
        println!("no document holds every word of {:?}; these hold some", args.query);
        println!();
    }
    for ranked in &answer.hits {
        print_ranked(ranked, answer.all_words);
    }
    if let Some(caveat) = meaning::caveat(&meaning) {
        println!("{caveat}");
    }
    Ok(())
}

pub fn similar(args: SimilarArgs) -> Result<()> {
    let (text, example) = if args.example.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).context("reading the passage from standard input")?;
        (text, None)
    } else {
        let text = read_example(&args.example).with_context(|| format!("reading {}", args.example.display()))?;
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
    let (answer, meaning) = meaning::similar(&library, finder.as_ref(), &args.models.dir()?, &text, args.limit, example.as_deref())?;
    match meaning.state {
        State::Ready => {}
        State::NoModel => bail!(
            "search by example needs the model: `nooma model fetch` ({})",
            crate::model::size(ModelSpec::default_model().bytes())
        ),
        State::NoVectors => bail!("no vectors yet: `nooma index` computes them"),
    }

    if args.json {
        let out = serde_json::json!({
            "example": args.example,
            "hits": answer.hits,
            "meaning": meaning.to_json(),
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if answer.hits.is_empty() {
        println!("nothing in the library is close to this passage");
        println!();
    }
    for ranked in &answer.hits {
        print_ranked(ranked, answer.all_words);
    }
    if let Some(caveat) = meaning::caveat(&meaning) {
        println!("{caveat}");
    }
    Ok(())
}

/// The text of the file a passage is taken from, as the library reads it.
fn read_example(path: &Path) -> Result<String> {
    let opened = nooma_core::reading::read_path(path).map_err(|unread| match unread {
        Unread::NotADocument(what) => anyhow!("not a document nooma reads: {what}"),
        Unread::TooLarge(why) | Unread::Locked(why) | Unread::Unopened(why) | Unread::Failed(why) => anyhow!("{why}"),
        Unread::NoText(_) => anyhow!("a picture with no text in it"),
    })?;
    Ok(opened.document.chunks.iter().map(|chunk| chunk.text.as_str()).collect::<Vec<_>>().join("\n\n"))
}

/// One row of the list: what it is, which halves found it, where, and the
/// passage that answered.
fn print_ranked(ranked: &Ranked, all_words: bool) {
    let hit = &ranked.hit;
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
    println!("{heading} ({})", found_by(ranked, all_words));
    // A line is where an editor opens a note; a PDF is read by its pages; a
    // Word document, a book or a picture is found in by the headings above.
    // Text OCR read is said to be, since its words may be misread.
    let has_lines = DocKind::from_name(&hit.kind).is_some_and(DocKind::has_lines);
    let ocr = if hit.recognized { " · OCR" } else { "" };
    match hit.page {
        Some(page) => println!("  {} · p. {page}{ocr}", hit.path.display()),
        None if has_lines => println!("  {}:{}{ocr}", hit.path.display(), hit.line),
        None => println!("  {}{ocr}", hit.path.display()),
    }
    let fragment = hit.fragment.split_whitespace().collect::<Vec<_>>().join(" ");
    if !fragment.is_empty() {
        println!("  {fragment}");
    }
    println!();
}

/// Which halves found a document: `words`, `some words`, `meaning 0.84`, or
/// both; `meaning 0.84, another language` when the meaning found it in the
/// other language. The cosine is printed because it means the same from one
/// search to the next - how close, not just which is closer.
fn found_by(ranked: &Ranked, all_words: bool) -> String {
    let words = ranked.words.map(|_| if all_words { "words" } else { "some words" }.to_string());
    let meaning = ranked.meaning.map(|place| {
        let across = if ranked.across { ", another language" } else { "" };
        format!("meaning {:.2}{across}", place.score)
    });
    [words, meaning].into_iter().flatten().collect::<Vec<_>>().join(" + ")
}
